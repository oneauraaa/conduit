/**
 * Which OS the UI is running on.
 *
 * Only for *copy* — anything behavioural belongs in the Rust platform layer,
 * where the compiler enforces that all three sides exist. This is here because
 * a few strings genuinely differ ("menu bar" vs "system tray") and getting them
 * wrong makes the app feel ported rather than native.
 *
 * `?platform=` overrides it, so every variant can be reviewed from one machine
 * with `pnpm dev`.
 */
const override = new URLSearchParams(window.location.search).get("platform");

/** The webview's own idea of the OS, used when there is no override. */
function detect(): "macos" | "windows" | "linux" {
  const ua = navigator.userAgent;
  if (ua.includes("Windows")) return "windows";
  // Order matters: WebKitGTK's user agent carries "Linux" and Chrome on macOS
  // does not, but an Android-style UA would carry both. conduit is desktop
  // only, so "Linux and not Mac" is enough.
  if (ua.includes("Linux") && !ua.includes("Mac OS X")) return "linux";
  return "macos";
}

export const platform: "macos" | "windows" | "linux" =
  override === "windows" || override === "linux" || override === "macos"
    ? override
    : detect();

export const isWindows = platform === "windows";
export const isLinux = platform === "linux";

/** What the OS calls the place conduit hides to. */
export const trayName = isWindows
  ? "the system tray"
  : isLinux
    ? "the system tray"
    : "the menu bar";

/** Where the pill parks: above the taskbar, the panel, or the Dock. */
export const dockName = isWindows
  ? "the taskbar"
  : isLinux
    ? "the panel"
    : "the Dock";

/** "this mac" / "this pc" — used in prose about the local machine. */
export const machineName = isWindows
  ? "this pc"
  : isLinux
    ? "this machine"
    : "this mac";
