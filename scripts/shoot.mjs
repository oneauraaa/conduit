// Screenshots the running dev server at the app's real window size, so the UI
// can be reviewed without building the Tauri shell.
//
// Usage: node scripts/shoot.mjs <outDir> [<name>:<query> ...]

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { resolve } from "node:path";
import { tmpdir } from "node:os";

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const [outDir, ...shots] = process.argv.slice(2);
mkdirSync(outDir, { recursive: true });

for (const shot of shots) {
  const [name, query = ""] = shot.split(/:(.*)/);
  const dark = query.includes("theme=dark");
  const profile = mkdtempSync(resolve(tmpdir(), "conduit-shot-"));

  try {
    execFileSync(
      CHROME,
      [
        "--headless",
        "--disable-gpu",
        "--hide-scrollbars",
        "--force-device-scale-factor=2",
        "--virtual-time-budget=3000",
        // The window is transparent with rounded corners, so give it a
        // desktop-ish backdrop rather than letting the corners bleed to white.
        `--default-background-color=${dark ? "1A2233" : "E9EEF5"}`,
        `--user-data-dir=${profile}`,
        `--screenshot=${resolve(outDir, `${name}.png`)}`,
        "--window-size=740,540",
        `http://localhost:1420/?${query}`,
      ],
      { stdio: ["ignore", "ignore", "ignore"], timeout: 60_000 },
    );
  } catch {
    // Chrome's exit code is unreliable; the file on disk is the real signal.
  } finally {
    rmSync(profile, { recursive: true, force: true });
  }
  console.log(`  ${name}.png`);
}
