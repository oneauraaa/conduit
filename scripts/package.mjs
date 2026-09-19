// Builds the native release asset GitHub publishes for this host.
//
// Windows ships the bare executable. Tauri embeds the frontend and app manifest
// into it; Windows 11 already supplies the separate WebView2 runtime.
//
// Linux ships Tauri's AppImage rather than a host-linked bare ELF binary.
//
// macOS ships `conduit.app.zip`. A .app is a directory, so GitHub cannot attach
// it directly. `ditto` preserves the bundle's symlinks and extended attributes.

import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const outDir = join(root, "dist-release");
const tag = process.env.CONDUIT_RELEASE_TAG?.trim() || `v${version}`;

if (!/^v[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:[-+][0-9A-Za-z.-]+)?$/.test(tag)) {
  console.error(`CONDUIT_RELEASE_TAG is not a safe release tag: ${tag}`);
  process.exit(1);
}

const isWindows = process.platform === "win32";
const isMac = process.platform === "darwin";
const isLinux = process.platform === "linux";

if (!isWindows && !isMac && !isLinux) {
  console.error(
    `conduit packages for windows, macos and linux; this is ${process.platform}`,
  );
  process.exit(1);
}

/** Passed through to `tauri build`, e.g. `--target universal-apple-darwin`.
 *  A bare `--` survives some package-manager invocations; drop it. */
const extraArgs = process.argv.slice(2).filter((a) => a !== "--");

function run(cmd, args, opts = {}) {
  // `pnpm` is a .cmd shim on Windows, so it needs the real name rather than a
  // shell. Spawning through a shell would also concatenate these arguments
  // unescaped, which Node now warns about.
  const exe = isWindows && cmd === "pnpm" ? "pnpm.cmd" : cmd;
  console.log(`\n$ ${exe} ${args.join(" ")}`);
  execFileSync(exe, args, { cwd: root, stdio: "inherit", ...opts });
}

mkdirSync(outDir, { recursive: true });

/* ── build ── */

if (isWindows) {
  // --no-bundle: there is no bundler target we want. It still builds the
  // release binary and still runs beforeBuildCommand, so the frontend is fresh.
  run("pnpm", ["tauri", "build", "--no-bundle", ...extraArgs]);
} else if (isLinux) {
  run("pnpm", ["tauri", "build", "--bundles", "appimage", ...extraArgs]);
} else {
  run("pnpm", ["tauri", "build", "--bundles", "app", ...extraArgs]);
}

/* ── locate what was built ── */

// `--target x` nests the output under target/<triple>/release.
const targetFlag = extraArgs.indexOf("--target");
const triple = targetFlag === -1 ? null : extraArgs[targetFlag + 1];
const releaseDir = triple
  ? join(root, "src-tauri", "target", triple, "release")
  : join(root, "src-tauri", "target", "release");

const arch = triple ?? (process.arch === "arm64" ? "aarch64" : "x64");

let payload;
let assetName;

if (isWindows) {
  payload = join(releaseDir, "conduit.exe");
  assetName = `conduit-${tag}-windows-${arch}.exe`;
} else if (isLinux) {
  const appImageDir = join(releaseDir, "bundle", "appimage");
  const candidates = existsSync(appImageDir)
    ? readdirSync(appImageDir).filter((name) => name.endsWith(".AppImage"))
    : [];
  if (candidates.length !== 1) {
    console.error(`expected one AppImage in ${appImageDir}, found ${candidates.length}`);
    process.exit(1);
  }
  payload = join(appImageDir, candidates[0]);
  assetName = `conduit-${tag}-linux-${arch}.AppImage`;
} else {
  payload = join(releaseDir, "bundle", "macos", "conduit.app");
  assetName = `conduit-${tag}-macos-universal.app.zip`;
}

if (!existsSync(payload)) {
  console.error(`\nbuild finished but ${payload} is missing`);
  process.exit(1);
}

/* ── stage the release asset ── */

const assetPath = join(outDir, assetName);
rmSync(assetPath, { force: true });

if (isWindows) {
  copyFileSync(payload, assetPath);
} else if (isLinux) {
  copyFileSync(payload, assetPath);
} else {
  // --keepParent so the archive contains conduit.app rather than its innards.
  run("ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", payload, assetPath]);
}

const mb = (statSync(assetPath).size / 1024 / 1024).toFixed(1);
console.log(`\n${assetName}  (${mb} MB)\n${assetPath}`);
