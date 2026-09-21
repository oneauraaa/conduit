import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { GTK_HOOK } from "./verify-appimage.mjs";

const X11_BACKEND = /^export GDK_BACKEND=x11\b.*$/m;
const WAYLAND_BACKEND = /^export GDK_BACKEND=wayland\b/m;

export function patchGtkLauncher(appDir) {
  const hookPath = join(appDir, GTK_HOOK);
  const launcher = readFileSync(hookPath, "utf8");

  if (WAYLAND_BACKEND.test(launcher) && !X11_BACKEND.test(launcher)) {
    return false;
  }
  if (!X11_BACKEND.test(launcher)) {
    throw new Error(`could not find LinuxDeploy's X11 override in ${hookPath}`);
  }

  writeFileSync(
    hookPath,
    launcher.replace(
      X11_BACKEND,
      "export GDK_BACKEND=wayland # Conduit's GTK layer-shell UI requires Wayland",
    ),
  );
  return true;
}

function appImagePluginPath() {
  const cacheRoot = process.env.XDG_CACHE_HOME?.trim() || join(homedir(), ".cache");
  return join(cacheRoot, "tauri", "linuxdeploy-plugin-appimage.AppImage");
}

function appImageArch() {
  if (process.arch === "x64") return "x86_64";
  if (process.arch === "arm64") return "aarch64";
  throw new Error(`unsupported AppImage architecture: ${process.arch}`);
}

export function repackAppImage(appImagePath, version) {
  const absolutePath = resolve(appImagePath);
  const pluginPath = appImagePluginPath();
  if (!existsSync(pluginPath)) {
    throw new Error(`Tauri's AppImage plugin is missing: ${pluginPath}`);
  }

  const workRoot = mkdtempSync(join(tmpdir(), "conduit-appimage-repack-"));
  const extractRoot = join(workRoot, "extract");
  const stagedPath = join(
    dirname(absolutePath),
    `.conduit-repacked-${process.pid}-${basename(absolutePath)}`,
  );
  mkdirSync(extractRoot);

  try {
    execFileSync(absolutePath, ["--appimage-extract"], {
      cwd: extractRoot,
      stdio: ["ignore", "ignore", "pipe"],
    });
    patchGtkLauncher(join(extractRoot, "squashfs-root"));

    execFileSync(
      pluginPath,
      [
        "--appimage-extract-and-run",
        `--appdir=${join(extractRoot, "squashfs-root")}`,
      ],
      {
        cwd: dirname(absolutePath),
        env: {
          ...process.env,
          ARCH: appImageArch(),
          LDAI_OUTPUT: stagedPath,
          LINUXDEPLOY_OUTPUT_VERSION: version,
        },
        stdio: "inherit",
      },
    );

    renameSync(stagedPath, absolutePath);
  } finally {
    rmSync(stagedPath, { force: true });
    rmSync(workRoot, { recursive: true, force: true });
  }
}
