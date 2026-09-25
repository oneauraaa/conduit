/**
 * Fallback data for running the UI outside the Tauri shell (`pnpm dev` in a
 * browser). Lets the interface be designed and reviewed without booting the
 * Rust core. Inside the real app none of this is ever reached.
 */

import { isLinux, isWindows } from "./platform";
import type {
  AgentTarget,
  BrowserState,
  ControlState,
  HyprlandState,
  Keybind,
  SandboxesState,
  Readiness,
  ServerState,
  Settings,
  TailscaleState,
  ToolDef,
} from "./types";

export const isStandalone = !("__TAURI_INTERNALS__" in window);

export const server: ServerState = {
  status: "running",
  port: 6767,
  startedAt: Date.now() - 1000 * 60 * 8 - 1000 * 12,
  lastError: null,
};

/**
 * Which readiness card to preview. All three are structurally different, so
 * `?platform=windows|linux|macos` (see `lib/platform.ts`) is the only way to
 * review one from a machine running another, without a rebuild.
 *
 * Each is set to the state where its card has the most to say, rather than to
 * a fully-green one — a card that only ever renders "everything is fine" is the
 * one that gets shipped broken.
 */
export const readiness: Readiness = isWindows
  ? {
      platform: "windows",
      // Not elevated, with an elevated window in front: the one combination
      // where the card has something real to say.
      elevated: false,
      elevatedForeground: true,
      dpiAware: true,
      captureSupported: true,
      borderlessCapture: true,
    }
  : isLinux
    ? {
        platform: "linux",
        desktop: "KDE",
        wayland: true,
        // Granted and capturing, but with the two opt-in capabilities off —
        // which is exactly what a fresh plasma install looks like.
        portalReady: true,
        portalError: null,
        // Plasma's portal carries input too, so this route matches it.
        inputRoute: "portal",
        inputReady: true,
        inputError: null,
        captureReady: true,
        windowManagement: true,
        accessibilityTree: false,
        accessibilityHint:
          "no application is publishing an accessibility tree. turn it on with:\n  gsettings set org.gnome.desktop.interface toolkit-accessibility true\nand add QT_ACCESSIBILITY=1 to ~/.config/environment.d/ for qt and kde apps, then log back in. screenshots and clicking work without it.",
        panicStop: false,
        panicStopHint:
          "hold-escape needs read access to the keyboard device. run:\n  sudo usermod -aG input you\nthen log out and back in. until then, the stop button on the pill is the way to take control back.",
      }
    : {
        platform: "macos",
        accessibility: true,
        screenRecording: false,
      };

export const settings: Settings = {
  defaultAccess: "full",
  toolsAccess: "all",
  toolToggles: {},
  port: 6767,
  theme: "system",
  remoteEnabled: false,
  remoteToken: "9f2c41ab77e0d5384b1e6ca90f37de52",
  corsEnabled: false,
  corsOrigins: [],
  startOnLogin: false,
  startHidden: false,
  browserAutoStart: true,
  outlineDesktop: true,
  outlineBrowser: false,
  browserPermissions: {
    openWebsites: "alwaysAllow",
    readHistory: "alwaysAllow",
    downloadFiles: "alwaysAsk",
    uploadFiles: "alwaysAsk",
  },
};

const browserDemo = isStandalone
  ? new URLSearchParams(window.location.search).get("browser")
  : null;

export const browser: BrowserState = {
  install: {
    status:
      browserDemo === "ready" || browserDemo === "visible"
        ? "ready"
        : browserDemo === "downloading"
          ? "downloading"
          : browserDemo === "error"
            ? "error"
            : "unavailable",
    expectedRevision: "playwright-mcp-0.0.80-chromium-1243",
    installedRevision:
      browserDemo === "ready" || browserDemo === "visible"
        ? "playwright-mcp-0.0.80-chromium-1243"
        : null,
    downloadedBytes: browserDemo === "downloading" ? 78_960000 : 0,
    totalBytes: 184320000,
    error: browserDemo === "error" ? "the download did not pass its integrity check" : null,
  },
  runStatus: browserDemo === "ready" || browserDemo === "visible" ? "running" : "stopped",
  mode: browserDemo === "visible" ? "visible" : "headless",
  selectedProfileId: "profile-default",
  profiles: [
    { id: "profile-default", name: "Default", incognito: false },
    { id: "profile-work", name: "Work", incognito: false },
    { id: "incognito", name: "Incognito", incognito: true },
  ],
  tabs:
    browserDemo === "ready" || browserDemo === "visible"
      ? [
          { index: 0, title: "Conduit Browser", url: "https://example.com", active: true },
          { index: 1, title: "Documentation", url: "https://playwright.dev", active: false },
        ]
      : [],
  downloads: [],
  preview: null,
  stopLatched: false,
  owner: browserDemo === "ready" ? "codex" : null,
};

const host = isWindows ? "desktop-7f2k1" : isLinux ? "cachyos-box" : "mac-studio";

export const tailscale: TailscaleState = {
  installed: true,
  connected: true,
  hostname: `${host}.tail9c2f1.ts.net`,
  sharing: true,
  publicUrl: `https://${host}.tail9c2f1.ts.net/9f2c41ab77e0d5384b1e6ca90f37de52/mcp`,
  error: null,
};

/**
 * A slice of a real Hyprland config, chosen for the cases that are easy to get
 * wrong rather than for coverage: a described bind, a chord bound twice, a
 * mouse bind, a bare media key, and one line the compositor rejected.
 *
 * Only on Linux — `available: false` is what every other platform reports, and
 * it is what grays the tab out, so `?platform=macos` reviews that state.
 */
function bind(
  mods: string[],
  key: string,
  dispatcher: string,
  args: string,
  extra: Partial<Keybind> = {},
): Keybind {
  return {
    id: `${mods.join("+")}-${key}-${dispatcher}`,
    mods,
    modAlias: mods.length ? "$mainMod" : null,
    key,
    dispatcher,
    args,
    description: null,
    flags: [],
    submap: null,
    section: null,
    sourceFile: "/home/you/.config/hypr/hyprland.conf",
    sourceLine: 1,
    active: true,
    alsoFires: [],
    ...extra,
  };
}

const S = ["SUPER"];
const sampleBinds: Keybind[] = [
  bind(S, "Q", "exec", "kitty", { section: "keybindings" }),
  bind(S, "C", "killactive", "", { section: "keybindings" }),
  bind(S, "E", "exec", "env QT_QPA_PLATFORMTHEME=kde dolphin", { section: "keybindings" }),
  bind(S, "R", "exec", "caelestia shell drawers toggle launcher", { section: "keybindings" }),
  // The same chord bound twice: hyprland runs both, top to bottom.
  bind(S, "L", "exec", "hyprlock", {
    section: "keybindings",
    alsoFires: ["global caelestia:lock"],
  }),
  bind(S, "L", "global", "caelestia:lock", { alsoFires: ["exec hyprlock"] }),
  bind(S, "left", "movefocus", "l", { section: "move focus with mainmod + arrow keys" }),
  bind(S, "right", "movefocus", "r", { section: "move focus with mainmod + arrow keys" }),
  bind(S, "1", "workspace", "1", { section: "switch workspaces with mainmod + [0-9]" }),
  bind(S, "2", "workspace", "2", { section: "switch workspaces with mainmod + [0-9]" }),
  bind(["SUPER", "SHIFT"], "1", "movetoworkspace", "1", {
    section: "move active window to a workspace",
  }),
  bind(S, "mouse:272", "movewindow", "", {
    section: "move/resize windows with mainmod + lmb/rmb",
    flags: ["mouse"],
  }),
  bind([], "XF86AudioRaiseVolume", "exec", "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+", {
    modAlias: null,
    section: "multimedia keys",
    flags: ["locked", "repeat"],
  }),
  bind([], "XF86AudioPlay", "exec", "playerctl play-pause", {
    modAlias: null,
    section: "multimedia keys",
    flags: ["locked"],
  }),
  bind(["SUPER", "SHIFT"], "P", "exec", "hyprshot -m region", {
    section: "screenshots",
    description: "select a region",
  }),
  bind(["SUPER", "CTRL"], "P", "exec", "hyprshot -m output", {
    section: "screenshots",
    description: "whole screen",
  }),
  // Declared in the config, rejected by the compositor.
  bind(["SUPER", "ALT"], "Z", "exec", "a-tool-that-does-not-exist", {
    section: "screenshots",
    active: false,
  }),
];

/** `?hypr=empty|error|mismatch` forces the states a healthy machine never shows. */
const hyprDemo = isStandalone
  ? new URLSearchParams(window.location.search).get("hypr")
  : null;

export const hyprland: HyprlandState = {
  available: isLinux,
  version: "v0.56.2",
  configPath: "/home/you/.config/hypr/hyprland.conf",
  configKind: "conf",
  activeConfigPath:
    hyprDemo === "mismatch"
      ? "/home/you/.config/hypr/hyprland.lua"
      : "/home/you/.config/hypr/hyprland.conf",
  mismatch:
    hyprDemo === "mismatch"
      ? "showing /home/you/.config/hypr/hyprland.conf, but hyprland is running /home/you/.config/hypr/hyprland.lua"
      : null,
  binds: hyprDemo === "empty" ? [] : sampleBinds,
  error: hyprDemo === "error" ? "could not run hyprctl: no such file or directory" : null,
};

export const control: ControlState = {
  phase: "idle",
  agent: null,
  mode: "auto",
  action: null,
  browserAction: false,
  stopped: false,
};

export const catalog: ToolDef[] = [
  { name: "screenshot", group: "vision", summary: "capture a display, or a region of one", risky: false },
  { name: "list_displays", group: "vision", summary: "enumerate displays with their bounds", risky: false },
  { name: "move_cursor", group: "input", summary: "glide the cursor to a point", risky: false },
  { name: "click", group: "input", summary: "click, double-click or right-click", risky: false },
  { name: "drag", group: "input", summary: "press, move, release — for sliders and drag-and-drop", risky: false },
  { name: "scroll", group: "input", summary: "scroll the surface under the cursor", risky: false },
  { name: "type_text", group: "input", summary: "type a string, unicode included", risky: false },
  { name: "key_press", group: "input", summary: "press a key with optional modifiers", risky: false },
  { name: "get_cursor_position", group: "input", summary: "read where the cursor is now", risky: false },
  { name: "list_windows", group: "windows", summary: "every on-screen window with its bounds", risky: false },
  { name: "focus_window", group: "windows", summary: "bring a window to the front", risky: false },
  { name: "set_window_bounds", group: "windows", summary: "move or resize a window", risky: false },
  { name: "list_apps", group: "windows", summary: "running applications", risky: false },
  { name: "open_app", group: "windows", summary: "launch or activate an application", risky: false },
  { name: "quit_app", group: "windows", summary: "ask an application to quit", risky: true },
  { name: "read_screen_text", group: "accessibility", summary: "read on-screen text and control bounds from the accessibility tree", risky: false },
  { name: "find_element", group: "accessibility", summary: "locate a control by its label and get a point to click", risky: false },
  { name: "web_search", group: "system", summary: "search the web with DuckDuckGo", risky: false },
  { name: "clipboard_read", group: "system", summary: "read the clipboard's text", risky: false },
  { name: "clipboard_write", group: "system", summary: "replace the clipboard's text", risky: true },
  { name: "run_shell", group: "system", summary: "run a shell command and capture its output", risky: true },
  { name: "wait", group: "system", summary: "pause, to let the ui settle", risky: false },
  { name: "notify", group: "system", summary: "post a notification", risky: false },
  { name: "list_keybinds", group: "system", summary: "the keyboard shortcuts the user has bound", risky: false },
];

export const browserCatalog: ToolDef[] = [
  { name: "browser_navigate", group: "browser", summary: "navigate the active page to a URL", risky: false },
  { name: "browser_navigate_back", group: "browser", summary: "go back in the active page", risky: false },
  { name: "browser_snapshot", group: "browser", summary: "read an accessibility snapshot of the page", risky: false },
  { name: "browser_find", group: "browser", summary: "find text in the current page", risky: false },
  { name: "browser_click", group: "browser", summary: "click an element from a snapshot", risky: false },
  { name: "browser_type", group: "browser", summary: "type into an editable element", risky: false },
  { name: "browser_fill_form", group: "browser", summary: "fill several form fields", risky: false },
  { name: "browser_hover", group: "browser", summary: "hover over an element", risky: false },
  { name: "browser_drag", group: "browser", summary: "drag one page element to another", risky: false },
  { name: "browser_drop", group: "browser", summary: "drop data or approved files on an element", risky: false },
  { name: "browser_select_option", group: "browser", summary: "choose one or more select options", risky: false },
  { name: "browser_press_key", group: "browser", summary: "press a keyboard key in the page", risky: false },
  { name: "browser_handle_dialog", group: "browser", summary: "accept or dismiss a browser dialog", risky: false },
  { name: "browser_file_upload", group: "browser", summary: "upload approved local files", risky: false },
  { name: "browser_take_screenshot", group: "browser", summary: "capture the current web page", risky: false },
  { name: "browser_wait_for", group: "browser", summary: "wait for time or page text", risky: false },
  { name: "browser_tabs", group: "browser", summary: "list, create, close, or select tabs", risky: false },
  { name: "browser_resize", group: "browser", summary: "resize the browser viewport", risky: false },
  { name: "browser_close", group: "browser", summary: "close the active page", risky: true },
  { name: "browser_history", group: "browser", summary: "search Conduit navigation history", risky: false },
];

const home = isWindows ? "C:/Users/you" : "/Users/you";
const desktopCfg = isWindows
  ? `${home}/AppData/Roaming/Claude/claude_desktop_config.json`
  : `${home}/Library/Application Support/Claude/claude_desktop_config.json`;

export const agents: AgentTarget[] = [
  { id: "claude-code", name: "claude code", configPath: `${home}/.claude.json`, detected: true, installed: true, installedTargets: ["host", "work"], error: null, icon: null },
  { id: "codex", name: "codex", configPath: `${home}/.codex/config.toml`, detected: true, installed: false, installedTargets: [], error: null, icon: null },
  { id: "opencode", name: "opencode", configPath: `${home}/.config/opencode/opencode.jsonc`, detected: true, installed: false, installedTargets: [], error: null, icon: null },
  { id: "claude-desktop", name: "claude desktop", configPath: desktopCfg, detected: true, installed: false, installedTargets: [], error: null, icon: null },
  { id: "hermes", name: "hermes agent", configPath: `${home}/.hermes/config.yaml`, detected: false, installed: false, installedTargets: [], error: null, icon: null },
  { id: "openclaw", name: "openclaw", configPath: `${home}/.openclaw/openclaw.json`, detected: false, installed: false, installedTargets: [], error: null, icon: null },
];

/**
 * Two sandboxes in the two states the tab has most to show for: one running
 * with a live view, one still building its first image. Mutable, so create,
 * stop and delete can be exercised in `pnpm dev`.
 */
export const sandboxes: SandboxesState = {
  docker: {
    availability: "ready",
    cliPath: isWindows
      ? "C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe"
      : "/usr/local/bin/docker",
    clientVersion: "27.3.1",
    serverVersion: "27.3.1",
    engine: isLinux ? "Ubuntu 24.04 LTS" : "Docker Desktop",
    arch: "aarch64",
    cpus: 8,
    memoryBytes: 8 * 1024 ** 3,
    hint: null,
  },
  sandboxes: [
    {
      spec: {
        id: "work",
        name: "work",
        os: "ubuntu-24.04",
        memoryMb: 4096,
        cpus: 2,
        width: 1280,
        height: 800,
        internet: true,
        autoStart: true,
        createdAt: Date.now() - 1000 * 60 * 60 * 26,
      },
      status: "running",
      error: null,
      build: null,
      paused: false,
      endpoint: "http://127.0.0.1:6767/sandbox/work/mcp",
      busyCalls: 0,
    },
    {
      spec: {
        id: "offline-lab",
        name: "offline lab",
        os: "debian-12",
        memoryMb: 2048,
        cpus: 1,
        width: 1440,
        height: 900,
        internet: false,
        autoStart: false,
        createdAt: Date.now() - 1000 * 60 * 4,
      },
      status: "building",
      error: null,
      build: {
        os: "debian-12",
        step: 4,
        totalSteps: 8,
        lastLine: "#8 41.2 Unpacking xfce4-panel (4.18.2-1) ...",
        startedAt: Date.now() - 1000 * 95,
      },
      paused: false,
      endpoint: "http://127.0.0.1:6767/sandbox/offline-lab/mcp",
      busyCalls: 0,
    },
  ],
  stopOnQuit: true,
};
