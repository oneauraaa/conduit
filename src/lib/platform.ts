/**
 * Which OS the UI is running on.
 *
 * Only for *copy* — anything behavioural belongs in the Rust platform layer,
 * where the compiler enforces that both sides exist. This is here because a few
 * strings genuinely differ ("menu bar" vs "system tray") and getting them wrong
 * makes the app feel ported rather than native.
 *
 * `?platform=` overrides it, so both variants can be reviewed from one machine
 * with `pnpm dev`.
 */
const override = new URLSearchParams(window.location.search).get("platform");

export const isWindows =
  override === "windows" ||
  (override === null && navigator.userAgent.includes("Windows"));

/** What the OS calls the place conduit hides to. */
export const trayName = isWindows ? "the system tray" : "the menu bar";

/** Where the pill parks: above the taskbar, or above the Dock. */
export const dockName = isWindows ? "the taskbar" : "the Dock";

/** "this mac" / "this pc" — used in prose about the local machine. */
export const machineName = isWindows ? "this pc" : "this mac";
