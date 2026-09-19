#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { createHash } = require("node:crypto");
const { spawnSync } = require("node:child_process");
const archiver = require("archiver");
const esbuild = require("esbuild");
const { chromium } = require("playwright");

const REVISION = "playwright-mcp-0.0.80-chromium-1243";
const root = __dirname;
const releaseRoot = path.join(root, "release");

function detectedTarget() {
  if (process.platform === "win32" && process.arch === "x64") return "windows-x64";
  if (process.platform === "linux" && process.arch === "x64") return "linux-x64";
  if (process.platform === "darwin" && process.arch === "x64") return "macos-x64";
  if (process.platform === "darwin" && process.arch === "arm64") return "macos-arm64";
  throw new Error("unsupported browser bundle host: " + process.platform + "-" + process.arch);
}

function pkgTarget(target) {
  return {
    "windows-x64": "node22-win-x64",
    "linux-x64": "node22-linux-x64",
    "macos-x64": "node22-macos-x64",
    "macos-arm64": "node22-macos-arm64",
  }[target];
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit", ...options });
  if (result.status !== 0) throw new Error(command + " exited with " + result.status);
}

function browserRoot(executable) {
  let current = path.dirname(executable);
  while (path.dirname(current) !== current) {
    if (/^chromium-\d+/.test(path.basename(current))) return current;
    current = path.dirname(current);
  }
  throw new Error("could not identify the Playwright Chromium directory");
}

function zipDirectory(source, destination) {
  return new Promise((resolve, reject) => {
    const output = fs.createWriteStream(destination);
    const archive = archiver("zip", { zlib: { level: 9 } });
    output.on("close", resolve);
    output.on("error", reject);
    archive.on("error", reject);
    archive.pipe(output);
    archive.directory(source, false);
    archive.finalize();
  });
}

function sha256(file) {
  const hash = createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

function replaceExactly(source, needle, replacement) {
  const count = source.split(needle).length - 1;
  if (count !== 1) {
    throw new Error(`expected one Playwright bundle seam, found ${count}: ${needle}`);
  }
  return source.replace(needle, replacement);
}

async function main() {
  const target = process.env.CONDUIT_BROWSER_TARGET || detectedTarget();
  if (target !== detectedTarget()) {
    throw new Error("browser bundles must be built on their matching target host");
  }
  const playwrightCli = path.join(path.dirname(require.resolve("playwright/package.json")), "cli.js");
  run(process.execPath, [playwrightCli, "install", "chromium"]);
  const chromiumExecutable = chromium.executablePath();
  if (!fs.existsSync(chromiumExecutable)) throw new Error("Playwright did not install Chromium");

  fs.rmSync(releaseRoot, { recursive: true, force: true });
  const bundle = path.join(releaseRoot, "bundle");
  const bin = path.join(bundle, "bin");
  const browserDestination = path.join(bundle, "chromium");
  fs.mkdirSync(bin, { recursive: true });
  // Runtime extraction rejects archive symlinks. Dereference framework links
  // while assembling macOS bundles so the resulting ZIP has the same safe,
  // regular-file shape on every platform.
  fs.cpSync(browserRoot(chromiumExecutable), browserDestination, { recursive: true, dereference: true });

  const extension = process.platform === "win32" ? ".exe" : "";
  const sidecar = path.join(bin, "conduit-browser-sidecar" + extension);
  const bundledSidecar = path.join(releaseRoot, "sidecar-bundle.cjs");
  esbuild.buildSync({
    entryPoints: [path.join(root, "sidecar.cjs")],
    outfile: bundledSidecar,
    bundle: true,
    platform: "node",
    target: "node22",
    format: "cjs",
    external: ["chromium-bidi/*"],
  });
  const coreRoot = path.dirname(require.resolve("playwright-core/package.json"));
  const corePackage = fs.readFileSync(path.join(coreRoot, "package.json"), "utf8").trim();
  const browsers = fs.readFileSync(path.join(coreRoot, "browsers.json"), "utf8").trim();
  let bundledSource = fs.readFileSync(bundledSidecar, "utf8");
  bundledSource = replaceExactly(
    bundledSource,
    'packageJSON = require(import_path9.default.join(packageRoot, "package.json"));',
    `packageJSON = ${corePackage};`,
  );
  bundledSource = replaceExactly(
    bundledSource,
    'registry = new Registry(require(import_path20.default.join(packageRoot, "browsers.json")));',
    `registry = new Registry(${browsers});`,
  );
  fs.writeFileSync(bundledSidecar, bundledSource);
  const pkgCli = require.resolve("@yao-pkg/pkg/lib-es5/bin.js");
  run(process.execPath, [
    pkgCli,
    bundledSidecar,
    "--config",
    path.join(root, "pkg.config.cjs"),
    "--targets",
    pkgTarget(target),
    "--output",
    sidecar,
    "--compress",
    "GZip",
    "--sea",
  ]);

  const chromiumRelative = path.relative(bundle, path.join(browserDestination, path.relative(browserRoot(chromiumExecutable), chromiumExecutable)));
  run(sidecar, ["--self-test", "--chromium", path.join(bundle, chromiumRelative)]);

  const archiveName = "conduit-browser-" + target + ".zip";
  const archivePath = path.join(releaseRoot, archiveName);
  await zipDirectory(bundle, archivePath);
  const repository = process.env.GITHUB_REPOSITORY || "oneauraaa/conduit";
  const releaseTag = process.env.CONDUIT_BROWSER_RELEASE_TAG || `v${require("../package.json").version}`;
  const manifest = {
    revision: REVISION,
    target,
    archiveUrl: "https://github.com/" + repository + "/releases/download/" + releaseTag + "/" + archiveName,
    size: fs.statSync(archivePath).size,
    sha256: sha256(archivePath),
    sidecarPath: path.relative(bundle, sidecar).split(path.sep).join("/"),
    chromiumPath: chromiumRelative.split(path.sep).join("/"),
  };
  fs.writeFileSync(
    path.join(releaseRoot, "conduit-browser-" + target + ".json"),
    JSON.stringify(manifest, null, 2) + "\n",
  );
  process.stdout.write(JSON.stringify(manifest, null, 2) + "\n");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
