#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const readline = require("node:readline");
const { randomUUID } = require("node:crypto");
const { Client } = require("@modelcontextprotocol/sdk/client/index.js");
const { InMemoryTransport } = require("@modelcontextprotocol/sdk/inMemory.js");
const { createConnection } = require("@playwright/mcp");
const { chromium } = require("playwright");

const REVISION = "playwright-mcp-0.0.80-chromium-1243";
const ALLOWED_TOOLS = new Set([
  "browser_navigate",
  "browser_navigate_back",
  "browser_snapshot",
  "browser_find",
  "browser_click",
  "browser_type",
  "browser_fill_form",
  "browser_hover",
  "browser_drag",
  "browser_drop",
  "browser_select_option",
  "browser_press_key",
  "browser_handle_dialog",
  "browser_file_upload",
  "browser_take_screenshot",
  "browser_wait_for",
  "browser_tabs",
  "browser_resize",
  "browser_close",
]);

const state = {
  handshakeComplete: false,
  context: null,
  connection: null,
  client: null,
  clientTransport: null,
  serverTransport: null,
  profileId: null,
  outputDir: null,
  mcpOutputDir: null,
  headless: true,
  viewport: { width: 1280, height: 720 },
  previewVisible: false,
  activePage: null,
  cdp: null,
  frameAt: 0,
  previewFrame: null,
  previewFlushing: false,
  currentCall: null,
  approvedNavigationRequests: new WeakSet(),
  permissionWaiters: new Map(),
  pendingDownloads: new Set(),
  reservedDestinations: new Set(),
  closing: false,
  callQueue: Promise.resolve(),
};

function send(value) {
  process.stdout.write(JSON.stringify(value) + "\n");
}

// Preview frames are intentionally lossy. If Rust or the webview falls behind,
// retain only the newest unsent frame instead of building a stale base64 queue
// that consumes memory and makes the preview lag farther behind reality.
function queuePreview(frame) {
  state.previewFrame = frame;
  if (state.previewFlushing) return;
  state.previewFlushing = true;

  const flush = () => {
    const next = state.previewFrame;
    state.previewFrame = null;
    if (!next) {
      state.previewFlushing = false;
      return;
    }
    const writable = process.stdout.write(JSON.stringify({ event: "frame", frame: next }) + "\n");
    if (writable) setImmediate(flush);
    else process.stdout.once("drain", flush);
  };
  flush();
}

function previewFrameDue(previousAt, now) {
  return now - previousAt >= 100;
}

function reply(id, result) {
  send({ id, ok: true, result: result === undefined ? null : result });
}

function reject(id, error) {
  send({ id, ok: false, error: error instanceof Error ? error.message : String(error) });
}

function isAllowedUrl(raw) {
  if (raw === "about:blank") return true;
  try {
    const url = new URL(raw);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

function safeFilename(value) {
  const clean = path.basename(value || "download")
    .replace(/[<>:\"/\\|?*\u0000-\u001f]/g, "_")
    .slice(0, 180);
  return clean && clean !== "." && clean !== ".." ? clean : "download";
}

function uniqueDestination(filename, root = state.outputDir, reserved = state.reservedDestinations) {
  const parsed = path.parse(safeFilename(filename));
  let candidate = path.join(root, parsed.base);
  let suffix = 1;
  while (fs.existsSync(candidate) || reserved.has(candidate)) {
    candidate = path.join(root, parsed.name + "-" + suffix + parsed.ext);
    suffix += 1;
  }
  const relative = path.relative(root, candidate);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error("download destination escaped the Conduit download folder");
  }
  reserved.add(candidate);
  return candidate;
}

function outputPath(filename, root = state.outputDir) {
  if (!filename || path.isAbsolute(filename)) {
    throw new Error("browser output filenames must be relative");
  }
  const candidate = path.resolve(root, filename);
  const relative = path.relative(root, candidate);
  if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error("browser output filename escaped the Conduit output folder");
  }
  fs.mkdirSync(path.dirname(candidate), { recursive: true });
  return candidate;
}

function mcpDownloadPath(filename) {
  if (!state.mcpOutputDir) return null;
  // @playwright/mcp@0.0.80 saves every observed download to outputDir using
  // this sanitizer. Keep that automatic copy in isolated spool storage so it
  // can never bypass Conduit's approval-controlled promotion into Downloads.
  const value = String(filename);
  const sanitizePart = (part) => part.replace(
    /[\x00-\x2C\x2E-\x2F\x3A-\x40\x5B-\x60\x7B-\x7F]+/g,
    "-",
  );
  const separator = value.lastIndexOf(".");
  const sanitized = separator === -1
    ? sanitizePart(value)
    : `${sanitizePart(value.slice(0, separator))}.${sanitizePart(value.slice(separator + 1))}`;
  const root = path.resolve(state.mcpOutputDir);
  const candidate = path.resolve(root, sanitized);
  const relative = path.relative(root, candidate);
  // Never let an unusual suggested filename turn cleanup into deletion of the
  // spool root (or anything outside it).
  if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) return null;
  return candidate;
}

function relativeOutputLinks(result) {
  if (!result || !Array.isArray(result.content)) return result;
  const prefix = state.outputDir + path.sep;
  return {
    ...result,
    content: result.content.map((item) => item.type === "text" && typeof item.text === "string"
      ? { ...item, text: item.text.split(prefix).join("./") }
      : item),
  };
}

async function requestPermission(category, detail, agent) {
  const requestId = randomUUID();
  return new Promise((resolve) => {
    const timeout = setTimeout(() => {
      state.permissionWaiters.delete(requestId);
      resolve(false);
    }, 62_000);
    state.permissionWaiters.set(requestId, (allowed) => {
      clearTimeout(timeout);
      resolve(Boolean(allowed));
    });
    send({
      event: "permission",
      requestId,
      category,
      detail,
      agent: agent || null,
      tool: state.currentCall ? state.currentCall.name : null,
    });
  });
}

function tabs() {
  if (!state.context) return [];
  return state.context.pages().map((page, index) => ({
    index,
    title: page.__conduitTitle || "",
    url: page.url(),
    active: page === state.activePage,
  }));
}

async function emitTabs() {
  if (!state.context) return;
  for (const page of state.context.pages()) {
    try {
      page.__conduitTitle = await page.title();
    } catch {
      page.__conduitTitle = "";
    }
  }
  send({ event: "tabs", tabs: tabs() });
}

async function recordNavigation(page, frame) {
  if (frame !== page.mainFrame()) return;
  const url = frame.url();
  if (!isAllowedUrl(url) || url === "about:blank") return;
  // `framenavigated` fires at commit, often before the document's <title> is
  // available. Wait for the new DOM, and discard superseded redirect hops, so
  // the durable history entry describes the page the user actually reached.
  await page.waitForLoadState("domcontentloaded", { timeout: 5_000 }).catch(() => {});
  if (page.isClosed() || frame !== page.mainFrame() || frame.url() !== url) return;
  let title = "";
  try {
    title = await page.title();
  } catch {
    // A page can close between navigation and title lookup.
  }
  send({
    event: "history",
    history: {
      id: randomUUID(),
      profileId: state.profileId,
      url,
      title,
      visitedAt: Date.now(),
    },
  });
  await emitTabs();
}

async function disablePreview() {
  const cdp = state.cdp;
  state.cdp = null;
  state.previewFrame = null;
  if (!cdp) return;
  try {
    await cdp.send("Page.stopScreencast");
  } catch {
    // The page may already be gone.
  }
  try {
    await cdp.detach();
  } catch {
    // Detaching twice is harmless here.
  }
}

async function enablePreview() {
  await disablePreview();
  if (!state.previewVisible || !state.headless || !state.activePage || state.activePage.isClosed()) {
    return;
  }
  const cdp = await state.context.newCDPSession(state.activePage);
  state.cdp = cdp;
  cdp.on("Page.screencastFrame", async (frame) => {
    try {
      await cdp.send("Page.screencastFrameAck", { sessionId: frame.sessionId });
    } catch {
      return;
    }
    const now = Date.now();
    if (!previewFrameDue(state.frameAt, now) || cdp !== state.cdp) return;
    state.frameAt = now;
    queuePreview({
      data: frame.data,
      mimeType: "image/jpeg",
      width: state.viewport.width,
      height: state.viewport.height,
      at: now,
    });
  });
  await cdp.send("Page.startScreencast", {
    format: "jpeg",
    quality: 70,
    maxWidth: state.viewport.width,
    maxHeight: state.viewport.height,
    everyNthFrame: 1,
  });
}

async function setActivePage(page) {
  if (!page || page.isClosed() || page === state.activePage) return;
  state.activePage = page;
  await emitTabs();
  await enablePreview();
}

function navigationDetail(url) {
  return JSON.stringify({ destinationUrl: url });
}

async function backDestination() {
  if (!state.activePage || state.activePage.isClosed()) return null;
  const cdp = await state.context.newCDPSession(state.activePage);
  try {
    const history = await cdp.send("Page.getNavigationHistory");
    return history.entries[history.currentIndex - 1]?.url || null;
  } finally {
    await cdp.detach().catch(() => {});
  }
}

async function installNavigationGuard(context) {
  await context.route("**/*", async (route) => {
    const request = route.request();
    if (!request.isNavigationRequest()) {
      await route.continue();
      return;
    }
    const url = request.url();
    if (!isAllowedUrl(url)) {
      await route.abort("blockedbyclient");
      return;
    }
    // A popup's first request is emitted before Playwright creates its Frame,
    // and request.frame() intentionally throws in that state. Such a request
    // is still a top-level navigation and must pass this policy gate.
    try {
      const frame = request.frame();
      if (frame !== frame.page().mainFrame()) {
        await route.continue();
        return;
      }
    } catch {
      // Initial popup navigation: no frame exists yet.
    }
    const redirectedFrom = request.redirectedFrom();
    if (redirectedFrom && state.approvedNavigationRequests.has(redirectedFrom)) {
      state.approvedNavigationRequests.add(request);
      await route.continue();
      return;
    }
    if (!state.currentCall) {
      await route.continue();
      return;
    }
    if (state.currentCall.grants.delete("openWebsites")) {
      // A Rust-side approval authorizes this navigation and its redirect
      // chain, not every popup or second top-level navigation that a landing
      // page might trigger before the tool returns.
      state.approvedNavigationRequests.add(request);
      await route.continue();
      return;
    }
    const allowed = await requestPermission(
      "openWebsites",
      navigationDetail(url),
      state.currentCall.agent,
    );
    if (allowed) {
      state.approvedNavigationRequests.add(request);
      await route.continue();
    } else {
      await route.abort("blockedbyclient");
    }
  });
}

async function handleDownload(download) {
  // Keep the initiating call object even if Chromium finishes emitting the
  // download after another await. That lets the tool result report a denied
  // or failed download deterministically instead of relying on Playwright's
  // platform-specific click result.
  const ownerCall = state.currentCall;
  const id = randomUUID();
  const filename = safeFilename(download.suggestedFilename());
  const mcpCopy = mcpDownloadPath(download.suggestedFilename());
  const sourceUrl = download.url();
  const destination = uniqueDestination(filename);
  const agent = state.currentCall ? state.currentCall.agent : null;
  const detail = JSON.stringify({ sourceUrl, filename, destination });
  try {
    send({
      event: "download",
      download: { id, filename, sourceUrl, path: null, status: "waiting", error: null },
    });
    const allowed = await requestPermission("downloadFiles", detail, agent);
    if (!allowed) {
      await download.cancel().catch(() => {});
      await download.delete().catch(() => {});
      if (ownerCall) ownerCall.downloadError = `the download of ${filename} was denied`;
      send({
        event: "download",
        download: { id, filename, sourceUrl, path: null, status: "denied", error: null },
      });
      return;
    }
    try {
      await download.saveAs(destination);
      await download.delete().catch(() => {});
      send({
        event: "download",
        download: { id, filename: path.basename(destination), sourceUrl, path: destination, status: "complete", error: null },
      });
    } catch (error) {
      await download.delete().catch(() => {});
      if (ownerCall) ownerCall.downloadError = `the download of ${filename} failed: ${error}`;
      send({
        event: "download",
        download: { id, filename, sourceUrl, path: null, status: "error", error: String(error) },
      });
    }
  } finally {
    // Playwright MCP observes the same page event and saves its own copy. Its
    // artifact callback settles alongside ours, so yield once before removing
    // that isolated copy on both approval and denial.
    await new Promise((resolve) => setImmediate(resolve));
    if (mcpCopy) {
      await fs.promises.rm(mcpCopy, { force: true, maxRetries: 20, retryDelay: 50 });
    }
    state.reservedDestinations.delete(destination);
  }
}

function attachPage(page) {
  if (!state.activePage) state.activePage = page;
  page.on("framenavigated", (frame) => {
    if (frame === page.mainFrame()) {
      void recordNavigation(page, frame);
    }
  });
  page.on("load", () => {
    void emitTabs();
  });
  page.on("close", () => {
    if (state.activePage === page) {
      const remaining = state.context ? state.context.pages() : [];
      state.activePage = remaining[remaining.length - 1] || null;
      void enablePreview();
    }
    void emitTabs();
  });
  page.on("dialog", (dialog) => {
    send({ event: "dialog", kind: dialog.type(), message: dialog.message(), url: page.url() });
  });
  page.on("download", (download) => {
    const task = handleDownload(download).finally(() => state.pendingDownloads.delete(task));
    state.pendingDownloads.add(task);
  });
}

async function launch(message) {
  if (!state.handshakeComplete) throw new Error("complete the Chromium sidecar handshake first");
  if (state.context) throw new Error("Chromium is already running");
  if (!message.chromium || !fs.existsSync(message.chromium)) {
    throw new Error("the Chromium executable is missing");
  }
  fs.mkdirSync(message.userDataDir, { recursive: true });
  fs.mkdirSync(message.outputDir, { recursive: true });
  const pendingDir = path.join(message.userDataDir, ".pending-downloads");
  // A crash can leave unapproved spool bytes behind. They are never promoted
  // to Downloads, and the next launch removes them before Chromium can reuse
  // the profile.
  fs.rmSync(pendingDir, { recursive: true, force: true });
  const chromiumPendingDir = path.join(pendingDir, "chromium");
  const mcpOutputDir = path.join(pendingDir, "mcp");
  fs.mkdirSync(chromiumPendingDir, { recursive: true });
  fs.mkdirSync(mcpOutputDir, { recursive: true });
  state.profileId = message.profileId;
  state.outputDir = message.outputDir;
  state.mcpOutputDir = mcpOutputDir;
  state.headless = Boolean(message.headless);
  state.viewport = message.viewport || { width: 1280, height: 720 };
  state.closing = false;

  const context = await chromium.launchPersistentContext(message.userDataDir, {
    executablePath: message.chromium,
    headless: state.headless,
    chromiumSandbox: true,
    viewport: state.viewport,
    acceptDownloads: true,
    downloadsPath: chromiumPendingDir,
    // Routing is the final guard for privileged URL schemes. Blocking service
    // workers prevents an existing profile worker from bypassing that guard.
    serviceWorkers: "block",
    args: [
      "--disable-background-networking",
      "--disable-component-update",
      "--disable-default-apps",
      "--disable-sync",
      "--no-first-run",
      "--no-default-browser-check",
    ],
  });
  state.context = context;
  context.on("page", (page) => {
    attachPage(page);
    if (state.currentCall) void setActivePage(page);
  });
  context.on("close", () => {
    if (!state.closing) {
      send({ event: "crash", error: "Chromium closed unexpectedly" });
      // Exit so Rust drops every pending call and a later request can lazily
      // launch a fresh browser instead of reusing a dead context.
      setImmediate(() => process.exit(1));
    }
  });
  for (const page of context.pages()) attachPage(page);
  state.activePage = context.pages()[0] || null;
  await installNavigationGuard(context);

  const connection = await createConnection(
    {
      capabilities: ["core"],
      outputDir: mcpOutputDir,
      imageResponses: "allow",
      snapshot: { mode: "full" },
      allowUnrestrictedFileAccess: true,
      codegen: "none",
      timeouts: { action: 10_000, navigation: 60_000, expect: 10_000, settle: 500 },
    },
    async () => context,
  );
  const pair = InMemoryTransport.createLinkedPair();
  state.serverTransport = pair[0];
  state.clientTransport = pair[1];
  const client = new Client({ name: "conduit-browser-sidecar", version: "1.0.0" }, { capabilities: {} });
  await connection.connect(state.serverTransport);
  await client.connect(state.clientTransport);
  const listed = await client.listTools();
  const upstreamNames = new Set(listed.tools.map((tool) => tool.name));
  for (const name of ALLOWED_TOOLS) {
    if (!upstreamNames.has(name)) throw new Error("pinned browser tool is missing: " + name);
  }
  state.connection = connection;
  state.client = client;
  await emitTabs();
  return { revision: REVISION, tools: listed.tools.filter((tool) => ALLOWED_TOOLS.has(tool.name)) };
}

async function callTool(message) {
  if (!state.client || !state.context) throw new Error("Chromium is not running");
  if (!ALLOWED_TOOLS.has(message.name)) throw new Error("browser tool is not allowed: " + message.name);
  state.currentCall = {
    name: message.name,
    agent: message.agent || null,
    grants: new Set(Array.isArray(message.grants) ? message.grants : []),
    downloadError: null,
  };
  try {
    const toolArguments = { ...(message.arguments || {}) };
    if (message.name === "browser_navigate_back" && !state.currentCall.grants.has("openWebsites")) {
      const destination = await backDestination();
      if (destination && !isAllowedUrl(destination)) {
        throw new Error("browser navigation only allows http, https, and about:blank");
      }
      if (destination) {
        const allowed = await requestPermission(
          "openWebsites",
          navigationDetail(destination),
          state.currentCall.agent,
        );
        if (!allowed) throw new Error("browser back navigation was denied");
        state.currentCall.grants.add("openWebsites");
      }
    }
    if (
      (message.name === "browser_snapshot" || message.name === "browser_take_screenshot")
      && toolArguments.filename
    ) {
      toolArguments.filename = outputPath(toolArguments.filename);
    }
    if (message.name === "browser_tabs" && message.arguments && message.arguments.action === "select") {
      const page = state.context.pages()[message.arguments.index];
      if (!page) throw new Error("browser tab index is out of range");
      await setActivePage(page);
    }
    const result = await state.client.callTool({ name: message.name, arguments: toolArguments });
    if (message.name === "browser_resize" && !result.isError) {
      state.viewport = {
        width: Math.round(toolArguments.width),
        height: Math.round(toolArguments.height),
      };
      await enablePreview();
    }
    if (message.name === "browser_tabs" && message.arguments && message.arguments.action === "new") {
      const pages = state.context.pages();
      await setActivePage(pages[pages.length - 1]);
    }
    if (message.name === "browser_close" && (!state.activePage || state.activePage.isClosed())) {
      const pages = state.context.pages();
      state.activePage = pages[pages.length - 1] || null;
      await enablePreview();
    }
    await new Promise((resolve) => setTimeout(resolve, 25));
    await Promise.allSettled(Array.from(state.pendingDownloads));
    await emitTabs();
    if (state.currentCall.downloadError) {
      return relativeOutputLinks({
        ...result,
        isError: true,
        content: [
          ...(Array.isArray(result.content) ? result.content : []),
          { type: "text", text: state.currentCall.downloadError },
        ],
      });
    }
    return relativeOutputLinks(result);
  } finally {
    state.currentCall = null;
  }
}

async function stop() {
  state.closing = true;
  await disablePreview();
  for (const waiter of state.permissionWaiters.values()) waiter(false);
  state.permissionWaiters.clear();
  if (state.client) await state.client.close().catch(() => {});
  if (state.connection) await state.connection.close().catch(() => {});
  if (state.context) await state.context.close().catch(() => {});
  state.client = null;
  state.connection = null;
  state.context = null;
  state.activePage = null;
  state.reservedDestinations.clear();
}

async function handle(message) {
  const id = message.id;
  try {
    switch (message.type) {
      case "handshake":
        if (message.protocol !== 1 || message.revision !== REVISION) {
          throw new Error("incompatible Chromium sidecar protocol or revision");
        }
        state.handshakeComplete = true;
        reply(id, { protocol: 1, revision: REVISION });
        return;
      case "selfTest":
        if (!state.handshakeComplete) throw new Error("complete the Chromium sidecar handshake first");
        await selfTest(message.chromium);
        reply(id, { revision: REVISION });
        return;
      case "launch":
        reply(id, await launch(message));
        return;
      case "callTool":
        state.callQueue = state.callQueue.then(() => callTool(message), () => callTool(message));
        reply(id, await state.callQueue);
        return;
      case "setPreview":
        state.previewVisible = Boolean(message.visible);
        await enablePreview();
        reply(id, { visible: state.previewVisible && state.headless });
        return;
      case "permissionResult": {
        const waiter = state.permissionWaiters.get(message.requestId);
        if (waiter) {
          state.permissionWaiters.delete(message.requestId);
          waiter(Boolean(message.allowed));
        }
        return;
      }
      case "stop":
        await stop();
        reply(id, null);
        setImmediate(() => process.exit(0));
        return;
      default:
        throw new Error("unknown sidecar request: " + message.type);
    }
  } catch (error) {
    if (id) reject(id, error);
    else console.error(error);
  }
}

async function selfTest(chromiumPath) {
  const browser = await chromium.launch({
    executablePath: chromiumPath,
    headless: true,
    chromiumSandbox: true,
    args: ["--disable-background-networking", "--disable-component-update", "--no-first-run"],
  });
  const page = await browser.newPage();
  await page.goto("about:blank");
  await page.setContent("<title>Conduit Chromium self-test</title><p>ready</p>");
  const ok = (await page.title()) === "Conduit Chromium self-test";
  await browser.close();
  if (!ok) throw new Error("Chromium did not render the self-test page");
}

if (require.main === module) {
  const selfTestIndex = process.argv.indexOf("--self-test");
  if (selfTestIndex !== -1) {
    const chromiumIndex = process.argv.indexOf("--chromium");
    const chromiumPath = chromiumIndex === -1 ? null : process.argv[chromiumIndex + 1];
    if (!chromiumPath) {
      console.error("--self-test requires --chromium <path>");
      process.exitCode = 2;
    } else {
      selfTest(chromiumPath).catch((error) => {
        console.error(error);
        process.exitCode = 1;
      });
    }
  } else {
    readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on("line", (line) => {
      try {
        void handle(JSON.parse(line));
      } catch (error) {
        console.error("invalid sidecar request", error);
      }
    });
  }
}

module.exports = {
  ALLOWED_TOOLS,
  REVISION,
  isAllowedUrl,
  safeFilename,
  uniqueDestination,
  outputPath,
  previewFrameDue,
};
