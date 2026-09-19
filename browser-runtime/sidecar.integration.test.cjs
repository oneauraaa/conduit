"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const os = require("node:os");
const path = require("node:path");
const readline = require("node:readline");
const { spawn } = require("node:child_process");
const { chromium } = require("playwright");
const { REVISION, isAllowedUrl } = require("./sidecar.cjs");

const chromiumPath = process.env.CONDUIT_BROWSER_CHROMIUM || chromium.executablePath();
const chromiumInstalled = fs.existsSync(chromiumPath);

function fixtureServer() {
  const page = `<!doctype html>
    <title>Conduit browser fixture</title>
    <style>#drag-source,#drop-zone{width:160px;height:48px;border:1px solid;margin:8px}</style>
    <a id="second" href="/second">Second page</a>
    <a id="popup" target="_blank" href="/popup">Open popup</a>
    <a id="blocked-data" href="data:text/html,blocked">Blocked data URL</a>
    <button id="clicked" onclick="result.textContent='clicked'">Click me</button>
    <input id="typed" aria-label="Typed field">
    <input id="filled" aria-label="Filled field">
    <select id="choice" aria-label="Choice"><option value="one">One</option><option value="two">Two</option></select>
    <button id="hover" onmouseenter="result.textContent='hovered'">Hover me</button>
    <div id="drag-source" draggable="true">Drag source</div>
    <div id="drop-zone">Drop zone</div>
    <script>
      const result = document.createElement('output'); result.id='result'; document.body.append(result);
      dragSource = document.querySelector('#drag-source'); dropZone = document.querySelector('#drop-zone');
      dragSource.addEventListener('dragstart', e => e.dataTransfer.setData('text/plain','dragged'));
      dropZone.addEventListener('dragover', e => e.preventDefault());
      dropZone.addEventListener('drop', e => { e.preventDefault(); result.textContent=e.dataTransfer.getData('text/plain') || 'file-dropped'; });
    </script>
    <button id="dialog" onclick="prompt('Fixture prompt','initial')">Open dialog</button>
    <input id="file" type="file">
    <a id="download" download="fixture.txt" href="/download">Download fixture</a>`;
  const server = http.createServer((request, response) => {
    if (request.url === "/redirect") {
      response.writeHead(302, { location: "/second" });
      response.end();
      return;
    }
    if (request.url === "/download") {
      response.writeHead(200, { "content-type": "text/plain", "content-disposition": "attachment; filename=fixture.txt" });
      response.end("fixture download\n");
      return;
    }
    if (request.url === "/popup-on-load") {
      response.writeHead(200, { "content-type": "text/html" });
      response.end("<title>Popup launcher</title><script>addEventListener('load',()=>open('/popup'))</script>");
      return;
    }
    if (request.url === "/script-hop") {
      response.writeHead(200, { "content-type": "text/html" });
      response.end("<title>Script hop</title><script>location.href='/second'</script>");
      return;
    }
    response.writeHead(200, { "content-type": "text/html" });
    response.end(request.url === "/second" ? "<title>Second fixture page</title><p>second page ready</p>" : page);
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

function startSidecar(onEvent) {
  const packagedSidecar = process.env.CONDUIT_BROWSER_SIDECAR;
  const child = spawn(
    packagedSidecar || process.execPath,
    packagedSidecar ? [] : [path.join(__dirname, "sidecar.cjs")],
    {
    stdio: ["pipe", "pipe", "pipe"],
    },
  );
  let nextId = 1;
  const pending = new Map();
  let stderr = "";
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  readline.createInterface({ input: child.stdout, crlfDelay: Infinity }).on("line", (line) => {
    const message = JSON.parse(line);
    if (message.event) {
      onEvent(message, send);
      return;
    }
    const waiter = pending.get(message.id);
    if (!waiter) return;
    pending.delete(message.id);
    message.ok ? waiter.resolve(message.result) : waiter.reject(new Error(message.error));
  });
  function send(message) {
    child.stdin.write(JSON.stringify(message) + "\n");
  }
  function request(type, values = {}) {
    const id = String(nextId++);
    send({ id, type, ...values });
    return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
  }
  child.on("exit", (code) => {
    const error = new Error(`sidecar exited ${code}: ${stderr}`);
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
  });
  return { child, request, stderr: () => stderr };
}

function assertToolResult(result, name) {
  assert.equal(Boolean(result && result.isError), false, `${name}: ${JSON.stringify(result)}`);
  assert.ok(Array.isArray(result.content), `${name} returned no MCP content`);
}

function containsFile(root, filename) {
  if (!fs.existsSync(root)) return false;
  return fs.readdirSync(root, { withFileTypes: true }).some((entry) =>
    entry.isDirectory()
      ? containsFile(path.join(root, entry.name), filename)
      : entry.name === filename,
  );
}

async function removeTreeWhenReleased(root) {
  let lastError;
  for (let attempt = 0; attempt < 50; attempt += 1) {
    try {
      fs.rmSync(root, { recursive: true, force: true });
      return;
    } catch (error) {
      if (!["EBUSY", "EPERM", "ENOTEMPTY"].includes(error.code)) throw error;
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  throw lastError;
}

test("private sidecar runs all nineteen pinned tools against a local fixture", {
  skip: !chromiumInstalled && "install Playwright Chromium to run the integration test",
  timeout: 120_000,
}, async (t) => {
  const server = await fixtureServer();
  const address = server.address();
  const origin = `http://127.0.0.1:${address.port}`;
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "conduit-browser-test-"));
  const profile = path.join(root, "profile");
  const output = path.join(root, "output");
  const upload = path.join(root, "upload.txt");
  fs.writeFileSync(upload, "fixture upload\n");
  const events = [];
  let deniedDownloads = 1;
  const headless = process.env.CONDUIT_BROWSER_HEADLESS !== "false";
  const sidecar = startSidecar((message, send) => {
    events.push(message);
    if (message.event === "permission") {
      const denied = message.category === "downloadFiles" && deniedDownloads-- > 0;
      // A user can take time to answer. Keep the first denial pending long
      // enough for Playwright MCP's own download observer to finish, proving
      // that no copy reaches the user-visible output directory before Conduit
      // receives the decision.
      if (denied) {
        setTimeout(
          () => send({ type: "permissionResult", requestId: message.requestId, allowed: false }),
          250,
        );
      } else {
        send({ type: "permissionResult", requestId: message.requestId, allowed: true });
      }
    }
  });
  t.after(async () => {
    if (sidecar.child.exitCode === null) {
      await sidecar.request("stop").catch(() => {});
    }
    await new Promise((resolve) => server.close(resolve));
    fs.rmSync(root, { recursive: true, force: true });
  });

  await sidecar.request("handshake", { protocol: 1, revision: REVISION });
  const tested = await sidecar.request("selfTest", { chromium: chromiumPath });
  assert.equal(tested.revision, REVISION);
  const launched = await sidecar.request("launch", {
    chromium: chromiumPath,
    userDataDir: profile,
    outputDir: output,
    profileId: "fixture-profile",
    headless,
    viewport: { width: 900, height: 700 },
  });
  assert.equal(launched.tools.length, 19);

  const call = async (name, toolArguments = {}, grants = ["openWebsites", "uploadFiles"]) => {
    let result;
    try {
      result = await sidecar.request("callTool", { name, arguments: toolArguments, grants, agent: "test-agent" });
    } catch (error) {
      throw new Error(`${name} failed: ${error.message}; sidecar stderr: ${sidecar.stderr()}`);
    }
    assertToolResult(result, name);
    return result;
  };

  await call("browser_navigate", { url: origin });
  await sidecar.request("callTool", {
    name: "browser_click",
    arguments: { target: "#blocked-data" },
    grants: ["openWebsites"],
    agent: "test-agent",
  });
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.ok(
    events.filter((event) => event.event === "tabs").at(-1).tabs.every((tab) => isAllowedUrl(tab.url)),
    "a page click reached a blocked URL scheme",
  );
  if (headless) {
    await sidecar.request("setPreview", { visible: true });
    const deadline = Date.now() + 5_000;
    while (!events.some((event) => event.event === "frame") && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    assert.ok(events.some((event) => event.event === "frame"), "headless preview emitted no frame");
    await sidecar.request("setPreview", { visible: false });
  }
  await call("browser_snapshot");
  await call("browser_find", { text: "Click me" });
  await call("browser_click", { target: "#popup" }, []);
  assert.ok(events.some((event) => event.event === "permission" && event.category === "openWebsites"));
  await call("browser_close");
  await call("browser_click", { target: "#clicked" });
  await call("browser_type", { target: "#typed", text: "typed value" });
  await call("browser_fill_form", { fields: [{ target: "#filled", name: "Filled field", type: "textbox", value: "filled value" }] });
  await call("browser_hover", { target: "#hover" });
  await call("browser_drag", { startTarget: "#drag-source", endTarget: "#drop-zone" });
  await call("browser_drop", { target: "#drop-zone", data: { "text/plain": "dropped data" } });
  await call("browser_drop", { target: "#drop-zone", paths: [upload] });
  await call("browser_select_option", { target: "#choice", values: ["two"] });
  await call("browser_press_key", { key: "Tab" });
  await call("browser_click", { target: "#dialog" });
  await call("browser_handle_dialog", { accept: true, promptText: "accepted" });
  await call("browser_click", { target: "#file" });
  await call("browser_file_upload", { paths: [upload] });
  const screenshotResult = await call("browser_take_screenshot", { filename: "fixture.png", scale: "css" });
  await call("browser_wait_for", { text: "Click me" });
  await call("browser_resize", { width: 820, height: 620 });
  if (headless) {
    const previousFrames = events.filter((event) => event.event === "frame").length;
    await sidecar.request("setPreview", { visible: true });
    const deadline = Date.now() + 5_000;
    while (
      events.filter((event) => event.event === "frame").length === previousFrames
      && Date.now() < deadline
    ) {
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    const frame = events.filter((event) => event.event === "frame").at(-1)?.frame;
    assert.deepEqual(
      frame && { width: frame.width, height: frame.height },
      { width: 820, height: 620 },
    );
    await sidecar.request("setPreview", { visible: false });
  }
  await call("browser_tabs", { action: "list" });
  await call("browser_tabs", { action: "new", url: `${origin}/second` }, []);
  await call("browser_tabs", { action: "select", index: 0 });
  await call("browser_tabs", { action: "close", index: 1 });
  await call("browser_navigate", { url: `${origin}/redirect` });
  const backPermissionCount = events.filter(
    (event) => event.event === "permission" && event.category === "openWebsites",
  ).length;
  await call("browser_navigate_back", {}, []);
  const backPermissions = events.filter(
    (event) => event.event === "permission" && event.category === "openWebsites",
  );
  assert.ok(
    backPermissions.length > backPermissionCount,
    "back navigation did not request its destination",
  );
  assert.ok(JSON.parse(backPermissions.at(-1).detail).destinationUrl.startsWith(origin));
  const deniedDownloadClick = await sidecar.request("callTool", {
    name: "browser_click",
    arguments: { target: "#download" },
    grants: ["openWebsites"],
    agent: "test-agent",
  });
  assert.equal(deniedDownloadClick.isError, true, "denying the download should fail its triggering click");
  assert.ok(events.some((event) => event.event === "download" && event.download.status === "denied"));
  assert.equal(fs.existsSync(path.join(output, "fixture.txt")), false);
  assert.equal(
    containsFile(profile, "fixture.txt"),
    false,
    `denied download remained in temporary storage: ${JSON.stringify(fs.readdirSync(profile, { recursive: true }))}`,
  );
  assert.deepEqual(
    fs.readdirSync(path.join(profile, ".pending-downloads", "chromium")),
    [],
    "denied download bytes remained in Chromium spool storage",
  );
  await call("browser_click", { target: "#download" }, ["openWebsites"]);
  await call("browser_close");

  assert.ok(events.some((event) => event.event === "history"));
  assert.ok(events.some((event) => event.event === "history"
    && event.history.title === "Conduit browser fixture"));
  assert.ok(events.some((event) => event.event === "tabs"));
  assert.ok(events.some((event) => event.event === "download" && event.download.status === "complete"));
  assert.ok(events.some((event) => event.event === "permission" && event.category === "openWebsites"));
  assert.ok(events.some((event) => event.event === "permission" && event.category === "downloadFiles"));
  assert.ok(events.some((event) => event.event === "permission" && event.tool === "browser_click"));
  assert.ok(
    containsFile(output, "fixture.png"),
    `screenshot was not stored below outputDir: ${JSON.stringify(screenshotResult)}; files=${JSON.stringify(fs.readdirSync(output, { recursive: true }))}`,
  );
  assert.ok(fs.existsSync(path.join(output, "fixture.txt")));
});

test("a navigation approval covers redirects but not a new script navigation or popup", {
  skip: !chromiumInstalled && "install Playwright Chromium to run the integration test",
  timeout: 45_000,
}, async (t) => {
  const server = await fixtureServer();
  const address = server.address();
  const origin = `http://127.0.0.1:${address.port}`;
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "conduit-browser-grant-test-"));
  const events = [];
  const sidecar = startSidecar((message, send) => {
    events.push(message);
    if (message.event === "permission") {
      send({ type: "permissionResult", requestId: message.requestId, allowed: true });
    }
  });
  t.after(async () => {
    if (sidecar.child.exitCode === null) await sidecar.request("stop").catch(() => {});
    await new Promise((resolve) => server.close(resolve));
    fs.rmSync(root, { recursive: true, force: true });
  });

  await sidecar.request("handshake", { protocol: 1, revision: REVISION });
  await sidecar.request("launch", {
    chromium: chromiumPath,
    userDataDir: path.join(root, "profile"),
    outputDir: path.join(root, "output"),
    profileId: "grant-profile",
    headless: true,
    viewport: { width: 800, height: 600 },
  });
  const redirected = await sidecar.request("callTool", {
    name: "browser_navigate",
    arguments: { url: `${origin}/redirect` },
    grants: ["openWebsites"],
    agent: "test-agent",
  });
  assertToolResult(redirected, "browser_navigate");
  assert.equal(
    events.filter((event) => event.event === "permission").length,
    0,
    "an HTTP redirect should stay within its approved navigation chain",
  );

  const scriptHop = await sidecar.request("callTool", {
    name: "browser_navigate",
    arguments: { url: `${origin}/script-hop` },
    grants: ["openWebsites"],
    agent: "test-agent",
  });
  assertToolResult(scriptHop, "browser_navigate");
  assert.ok(
    events.some((event) => event.event === "permission"
      && JSON.parse(event.detail).destinationUrl === `${origin}/second`),
    "a direct navigation grant incorrectly authorized a client-side navigation",
  );

  events.length = 0;
  const result = await sidecar.request("callTool", {
    name: "browser_navigate",
    arguments: { url: `${origin}/popup-on-load` },
    grants: ["openWebsites"],
    agent: "test-agent",
  });
  assertToolResult(result, "browser_navigate");
  assert.ok(
    events.some((event) => event.event === "permission" && event.category === "openWebsites"),
    "a direct navigation grant incorrectly authorized a landing-page popup",
  );
});

test("an active browser request fails when the sidecar crashes", {
  skip: !chromiumInstalled && "install Playwright Chromium to run the integration test",
  timeout: 30_000,
}, async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "conduit-browser-crash-"));
  const sidecar = startSidecar((message, send) => {
    if (message.event === "permission") {
      send({ type: "permissionResult", requestId: message.requestId, allowed: true });
    }
  });
  // Windows can deliver the sidecar exit before Chromium has observed its
  // closed Playwright pipe and released profile files. Retry for a bounded
  // period; if the browser is actually orphaned, cleanup still fails.
  t.after(() => removeTreeWhenReleased(root));
  await sidecar.request("handshake", { protocol: 1, revision: REVISION });
  await sidecar.request("launch", {
    chromium: chromiumPath,
    userDataDir: path.join(root, "profile"),
    outputDir: path.join(root, "output"),
    profileId: "crash-profile",
    headless: true,
    viewport: { width: 800, height: 600 },
  });
  const pending = sidecar.request("callTool", {
    name: "browser_wait_for",
    arguments: { time: 10 },
    grants: [],
    agent: "test-agent",
  });
  await new Promise((resolve) => setTimeout(resolve, 100));
  sidecar.child.kill();
  await assert.rejects(pending, /sidecar exited/);
});
