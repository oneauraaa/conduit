import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const TRAY_LIBRARY = "libayatana-appindicator3.so.1";
export const GTK_HOOK = "apprun-hooks/linuxdeploy-plugin-gtk.sh";
const TRAY_DEPENDENCY_PREFIXES = ["libayatana-", "libdbusmenu-"];

function elfDependencies(libraryPath) {
  const dynamicSection = execFileSync("readelf", ["-d", libraryPath], {
    encoding: "utf8",
  });
  return [...dynamicSection.matchAll(/Shared library: \[([^\]]+)\]/g)].map(
    (match) => match[1],
  );
}

export function verifyAppImage(appImagePath) {
  const absolutePath = resolve(appImagePath);
  const extractRoot = mkdtempSync(join(tmpdir(), "conduit-appimage-verify-"));

  try {
    execFileSync(absolutePath, ["--appimage-extract"], {
      cwd: extractRoot,
      stdio: ["ignore", "ignore", "pipe"],
    });

    const appDir = join(extractRoot, "squashfs-root");
    const gtkHook = readFileSync(join(appDir, GTK_HOOK), "utf8");
    if (/^export GDK_BACKEND=x11\b/m.test(gtkHook)) {
      throw new Error("AppImage launcher forces X11 instead of Wayland");
    }
    if (!/^export GDK_BACKEND=wayland\b/m.test(gtkHook)) {
      throw new Error("AppImage launcher does not select the Wayland backend");
    }

    const libraryRoot = join(appDir, "usr", "lib");
    const missing = [];
    const visited = new Set();
    const pending = [TRAY_LIBRARY];

    while (pending.length > 0) {
      const library = pending.pop();
      if (visited.has(library)) continue;
      visited.add(library);

      const libraryPath = join(libraryRoot, library);
      try {
        statSync(libraryPath);
      } catch {
        missing.push(library);
        continue;
      }

      for (const dependency of elfDependencies(libraryPath)) {
        if (
          TRAY_DEPENDENCY_PREFIXES.some((prefix) =>
            dependency.startsWith(prefix),
          )
        ) {
          pending.push(dependency);
        }
      }
    }

    if (missing.length > 0) {
      throw new Error(
        `AppImage is missing its bundled tray runtime: ${missing.join(", ")}`,
      );
    }

    console.log(
      `verified AppImage tray runtime (${visited.size} libraries)`,
    );
  } finally {
    rmSync(extractRoot, { recursive: true, force: true });
  }
}

const invokedPath = process.argv[1];
if (
  invokedPath &&
  import.meta.url ===
    pathToFileURL(isAbsolute(invokedPath) ? invokedPath : resolve(invokedPath)).href
) {
  const appImagePath = process.argv[2];
  if (!appImagePath) {
    console.error("usage: node scripts/verify-appimage.mjs <path-to-AppImage>");
    process.exit(2);
  }

  try {
    verifyAppImage(appImagePath);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}
