// Builds conduit and puts it in a zip. No installer.
//
// Windows ships the bare executable: Tauri embeds the frontend and the app
// manifest into it, so `conduit.exe` is the whole app. The one thing it does
// not carry is the WebView2 runtime, which Windows 11 already has and which an
// installer would otherwise fetch — see the README note.
//
// macOS ships `conduit.app`, zipped with `ditto` rather than `zip`. A .app is a
// directory full of symlinks and extended attributes, and plain `zip` flattens
// both, which breaks the bundle (and any signature on it).
//
// Linux ships the bare ELF binary, like Windows. No .deb or AppImage: conduit
// links against the WebKitGTK and GTK already on any desktop that can run a
// browser, and its genuinely optional pieces (gtk-layer-shell) are dlopened at
// runtime rather than linked, so there is nothing for a package manager to
// resolve. `zip -j` because the binary must sit at the archive root.

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, existsSync, statSync } from "node:fs";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const outDir = join(root, "dist-release");

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

if (isWindows || isLinux) {
  // --no-bundle: there is no bundler target we want. It still builds the
  // release binary and still runs beforeBuildCommand, so the frontend is fresh.
  run("pnpm", ["tauri", "build", "--no-bundle", ...extraArgs]);
} else {
  // `app` is the .app bundle; dmg is deliberately not built.
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
let zipName;

if (isWindows) {
  payload = join(releaseDir, "conduit.exe");
  zipName = `conduit-${version}-windows-${arch}.zip`;
} else if (isLinux) {
  payload = join(releaseDir, "conduit");
  zipName = `conduit-${version}-linux-${arch}.zip`;
} else {
  payload = join(releaseDir, "bundle", "macos", "conduit.app");
  zipName = `conduit-${version}-macos-${arch}.zip`;
}

if (!existsSync(payload)) {
  console.error(`\nbuild finished but ${payload} is missing`);
  process.exit(1);
}

/* ── zip ── */

const zipPath = join(outDir, zipName);
rmSync(zipPath, { force: true });

if (isWindows) {
  // Compress-Archive is built in; no toolchain to install on a CI runner.
  run("powershell", [
    "-NoProfile",
    "-Command",
    `Compress-Archive -Path '${payload}' -DestinationPath '${zipPath}' -Force`,
  ]);
} else if (isLinux) {
  // -j drops the directory names, so the binary lands at the archive root.
  // Without it the zip carries the whole src-tauri/target/release path.
  run("zip", ["-j", "-9", zipPath, payload]);
} else {
  // --keepParent so the archive contains conduit.app rather than its innards.
  run("ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", payload, zipPath]);
}

const mb = (statSync(zipPath).size / 1024 / 1024).toFixed(1);
console.log(`\n${zipName}  (${mb} MB)\n${zipPath}`);
