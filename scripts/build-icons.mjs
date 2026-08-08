// Renders assets/*.svg to PNG using headless Chrome, then hands the app icon to
// `tauri icon` to produce the .icns and the full size ladder.
//
// Chrome is used because macOS ships no SVG rasterizer on the command line
// (sips and qlmanage both refuse SVG input) and we don't want a native
// image-processing dependency just to build three icons.
//
// Two Chrome quirks shape this script:
//   - Relaunching Chrome against a reused --user-data-dir aborts with
//     "Trying to load the allocator multiple times", so every render gets a
//     fresh throwaway profile directory.
//   - Chrome clamps windows to a minimum size, so tiny targets like the 22pt
//     tray glyph can't be screenshotted directly. We render big and let sips
//     downsample, which also gives cleaner edges than Chrome's rasterizer.

import { execFileSync } from "node:child_process";
import { mkdtempSync, existsSync, rmSync, statSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const CHROME_CANDIDATES = [
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/Applications/Chromium.app/Contents/MacOS/Chromium",
  "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
  "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];

const chrome = CHROME_CANDIDATES.find((p) => existsSync(p));
if (!chrome) {
  console.error(
    "No Chromium-based browser found. Install Google Chrome, or render\n" +
      "assets/icon.svg to a 1024x1024 assets/icon.png by hand and re-run\n" +
      "`pnpm tauri icon assets/icon.png`.",
  );
  process.exit(1);
}

function render(svg, png, size) {
  const out = resolve(root, png);
  rmSync(out, { force: true });

  const profile = mkdtempSync(resolve(tmpdir(), "conduit-icon-"));

  // Each SVG's declared width/height must match `size`. Chrome renders an SVG
  // at its own declared size and leaves the rest of the viewport transparent,
  // so a mismatch silently produces a small drawing in the corner rather than
  // a scaled one — which is exactly how the tray glyph broke once.
  try {
    execFileSync(
      chrome,
      [
        "--headless",
        "--disable-gpu",
        "--hide-scrollbars",
        "--force-device-scale-factor=1",
        "--virtual-time-budget=4000",
        "--default-background-color=00000000",
        `--user-data-dir=${profile}`,
        `--screenshot=${out}`,
        `--window-size=${size},${size}`,
        `file://${resolve(root, svg)}`,
      ],
      { stdio: ["ignore", "ignore", "ignore"], timeout: 60_000 },
    );
  } catch {
    // Chrome exits non-zero for reasons unrelated to the screenshot (profile
    // teardown, crashpad). The file on disk is the real success signal.
  } finally {
    rmSync(profile, { recursive: true, force: true });
  }

  if (!existsSync(out) || statSync(out).size === 0) {
    throw new Error(`failed to render ${svg} -> ${png}`);
  }
  console.log(`  ${png}  ${size}x${size}`);
}

function downsample(src, dest, size) {
  execFileSync(
    "sips",
    ["-z", String(size), String(size), resolve(root, src), "--out", resolve(root, dest)],
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  console.log(`  ${dest}  ${size}x${size}`);
}

console.log("rendering svg -> png");
render("assets/icon.svg", "assets/icon.png", 1024);

// Render the tray glyph oversized, then step it down to menu-bar sizes.
const trayScratch = "assets/.tray-4x.png";
render("assets/tray.svg", trayScratch, 352);
downsample(trayScratch, "assets/tray@2x.png", 44);
downsample(trayScratch, "assets/tray.png", 22);
rmSync(resolve(root, trayScratch), { force: true });

console.log("generating .icns + size ladder");
execFileSync("pnpm", ["tauri", "icon", "assets/icon.png"], {
  cwd: root,
  stdio: "inherit",
});

console.log("done");
