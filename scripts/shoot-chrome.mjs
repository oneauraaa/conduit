// Screenshots an arbitrary dev-server path at a given size.
// Usage: node scripts/shoot-chrome.mjs <out.png> <url-path> <w> <h> [bgHex]

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { tmpdir } from "node:os";

const CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const [out, path, w, h, bg = "00000000"] = process.argv.slice(2);
mkdirSync(dirname(resolve(out)), { recursive: true });

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
      `--default-background-color=${bg}`,
      `--user-data-dir=${profile}`,
      `--screenshot=${resolve(out)}`,
      `--window-size=${w},${h}`,
      `http://localhost:1420${path}`,
    ],
    { stdio: ["ignore", "ignore", "ignore"], timeout: 60_000 },
  );
} catch {
  // Chrome's exit code is unreliable; the file on disk is the real signal.
} finally {
  rmSync(profile, { recursive: true, force: true });
}
console.log(`  ${out}`);
