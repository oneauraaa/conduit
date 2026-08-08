/**
 * Fallback data for running the UI outside the Tauri shell (`pnpm dev` in a
 * browser). Lets the interface be designed and reviewed without booting the
 * Rust core. Inside the real app none of this is ever reached.
 */

import type {
  AgentTarget,
  ControlState,
  PermissionState,
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

export const permissions: PermissionState = {
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
};

export const tailscale: TailscaleState = {
  installed: true,
  connected: true,
  hostname: "mac-studio.tail9c2f1.ts.net",
  sharing: true,
  publicUrl:
    "https://mac-studio.tail9c2f1.ts.net/9f2c41ab77e0d5384b1e6ca90f37de52/mcp",
  error: null,
};

export const control: ControlState = {
  phase: "idle",
  agent: null,
  mode: "auto",
  action: null,
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
  { name: "clipboard_read", group: "system", summary: "read the clipboard's text", risky: false },
  { name: "clipboard_write", group: "system", summary: "replace the clipboard's text", risky: true },
  { name: "run_shell", group: "system", summary: "run a shell command and capture its output", risky: true },
  { name: "wait", group: "system", summary: "pause, to let the ui settle", risky: false },
  { name: "notify", group: "system", summary: "post a notification", risky: false },
];

export const agents: AgentTarget[] = [
  { id: "claude-code", name: "claude code", configPath: "/Users/you/.claude.json", detected: true, installed: true, error: null, icon: null },
  { id: "codex", name: "codex", configPath: "/Users/you/.codex/config.toml", detected: true, installed: false, error: null, icon: null },
  { id: "gemini", name: "gemini cli", configPath: "/Users/you/.gemini/settings.json", detected: true, installed: false, error: null, icon: null },
  { id: "opencode", name: "opencode", configPath: "/Users/you/.config/opencode/opencode.jsonc", detected: true, installed: false, error: null, icon: null },
  { id: "claude-desktop", name: "claude desktop", configPath: "/Users/you/Library/Application Support/Claude/claude_desktop_config.json", detected: true, installed: false, error: null, icon: null },
  { id: "hermes", name: "hermes agent", configPath: "/Users/you/.hermes/config.yaml", detected: false, installed: false, error: null, icon: null },
  { id: "openclaw", name: "openclaw", configPath: "/Users/you/.openclaw/openclaw.json", detected: false, installed: false, error: null, icon: null },
];
