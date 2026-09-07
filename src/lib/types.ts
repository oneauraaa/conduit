/** Shared shapes between the Rust core and every webview. Keep in sync with
 *  `src-tauri/src/state.rs` — these are the serde representations. */

export type ServerStatus = "stopped" | "starting" | "running" | "error";

export interface ServerState {
  status: ServerStatus;
  port: number;
  /** Unix millis the server came up; null while stopped. */
  startedAt: number | null;
  lastError: string | null;
}

/** How much the agent may do without stopping to ask. */
export type AccessMode = "manual" | "auto" | "full";

/** Master gate over the whole tool catalog. Only the Tools tab may change it. */
export type ToolsAccess = "all" | "custom" | "off";

export type ToolGroup = "vision" | "input" | "windows" | "accessibility" | "system";

export interface ToolDef {
  name: string;
  group: ToolGroup;
  summary: string;
  /** Prompts for confirmation in `auto` mode. */
  risky: boolean;
}

export interface Settings {
  defaultAccess: AccessMode;
  toolsAccess: ToolsAccess;
  /** tool name -> enabled. Absent means enabled. */
  toolToggles: Record<string, boolean>;
  port: number;
  theme: "light" | "dark" | "system";
  /** Whether the endpoint is published to the internet via Tailscale Funnel. */
  remoteEnabled: boolean;
  /** Secret path segment guarding the public endpoint. */
  remoteToken: string;
  /**
   * Whether browser-based clients may reach the endpoint at all. Off by
   * default — this endpoint can drive the whole machine and has no
   * authentication, so an open door here is a drive-by RCE.
   */
  corsEnabled: boolean;
  /** Exact origins allowed when `corsEnabled`. Empty allows nothing. */
  corsOrigins: string[];
  /** Launch conduit when the user logs in. */
  startOnLogin: boolean;
  /** When launched at login, go to the tray instead of showing the window. */
  startHidden: boolean;
}

export interface TailscaleState {
  installed: boolean;
  connected: boolean;
  /** MagicDNS name of this machine. */
  hostname: string | null;
  sharing: boolean;
  publicUrl: string | null;
  error: string | null;
}

/**
 * One of the user's own keyboard shortcuts.
 *
 * Read from two places and merged — see `src-tauri/src/hyprland.rs`. The
 * compositor decides what exists (`active`, `flags`, `submap`); the config file
 * supplies everything a human wrote (`description`, `section`, `modAlias`,
 * `sourceFile`).
 */
export interface Keybind {
  id: string;
  /** Resolved modifier names, in the order people say them. */
  mods: string[];
  /** The variable the config used for them, e.g. `$mainMod`. */
  modAlias: string | null;
  /** `Q`, `left`, `mouse:272`, `code:28`, `XF86AudioMute`. */
  key: string;
  dispatcher: string;
  args: string;
  /** A `bindd` description, or the trailing `#` comment on the line. */
  description: string | null;
  flags: string[];
  submap: string | null;
  /** The comment heading above it, used to group the list. */
  section: string | null;
  sourceFile: string | null;
  sourceLine: number | null;
  /** The compositor has this bind loaded. False means the config declares it
   *  but Hyprland does not have it — a rejected line, or an unsaved edit. */
  active: boolean;
  /** The other binds on this same chord. Hyprland runs every match rather than
   *  stopping at the first, so these all fire together. */
  alsoFires: string[];
}

export interface HyprlandState {
  /** This session is Hyprland. Everything else is meaningless when false. */
  available: boolean;
  version: string | null;
  /** The file the binds were read from. */
  configPath: string | null;
  configKind: "conf" | "lua" | null;
  /** The file Hyprland itself loaded, which is not always the one above. */
  activeConfigPath: string | null;
  /** Set when those two disagree: conduit prefers .conf, Hyprland prefers .lua. */
  mismatch: string | null;
  binds: Keybind[];
  /** One source failed. Never fatal alone — the other still fills the list. */
  error: string | null;
}

/**
 * What the machine needs from the user before conduit can do its job.
 *
 * A tagged union rather than one struct with optional fields, because the two
 * platforms genuinely differ: macOS withholds two capabilities behind TCC
 * grants, while Windows grants everything up front but has conditions that
 * silently change what works. The Server tab is where a user goes to find out
 * why something isn't working, so it has to tell the truth about which OS
 * they're on.
 */
export type Readiness =
  | {
      platform: "macos";
      accessibility: boolean;
      screenRecording: boolean;
    }
  | {
      platform: "windows";
      /** Running as administrator. Without it UIPI silently discards synthetic
       *  input aimed at any window owned by an elevated process. */
      elevated: boolean;
      /** An elevated window is in the foreground right now — the moment the
       *  above actually bites. */
      elevatedForeground: boolean;
      /** Per-monitor DPI aware v2, which is what makes physical-pixel
       *  coordinates coherent. */
      dpiAware: boolean;
      /** Windows.Graphics.Capture is present (Windows 10 1903+). */
      captureSupported: boolean;
      /** The capture session can suppress the yellow recording border
       *  (Windows 11 22000+). */
      borderlessCapture: boolean;
    }
  | {
      platform: "linux";
      /** `XDG_CURRENT_DESKTOP`, so the card can name what it found. */
      desktop: string;
      wayland: boolean;
      /** The screen-sharing session is live. This is about *capture*: input
       *  used to ride on the same portal session and no longer does, so a
       *  machine can see nothing and still drive the pointer perfectly.
       *  Wayland refuses to make this grant permanent, so it is asked every
       *  launch. */
      portalReady: boolean;
      /** Why it is not, when it is not. `null` while the prompt is unanswered. */
      portalError: string | null;
      /** Which of Wayland's two input routes this machine uses: "wlroots" for
       *  the virtual-pointer/virtual-keyboard protocols (hyprland, sway),
       *  "portal" for RemoteDesktop (gnome, kde). */
      inputRoute: "wlroots" | "portal";
      /** Input is usable: the pointer moves and keys land. */
      inputReady: boolean;
      /** Why it is not, when it is not. */
      inputError: string | null;
      /** Frames are actually arriving. A session can be live while capture is
       *  not, if the compositor negotiated a buffer type conduit cannot map. */
      captureReady: boolean;
      /** KWin is present, so windows can be listed, moved and focused. No
       *  Wayland protocol exposes this, so it is Plasma-only. */
      windowManagement: boolean;
      /** Some application is publishing an AT-SPI tree. Off by default. */
      accessibilityTree: boolean;
      accessibilityHint: string;
      /** The keyboard is readable, so hold-Escape works. */
      panicStop: boolean;
      panicStopHint: string;
    };

/** One row in the Server tab's live log. */
export interface ToolCallEvent {
  id: string;
  tool: string;
  client: string | null;
  at: number;
  outcome: "ok" | "denied" | "error" | "blocked";
  detail: string | null;
  durationMs: number | null;
}

/**
 * Pushed to the overlay on every step of a cursor tween. `x`/`y` are already
 * local to the receiving overlay's display — Rust subtracts the display origin
 * before emitting, so the overlay never has to know where its screen sits.
 */
export interface CursorEvent {
  x: number;
  y: number;
  /** Index of the display this event belongs to. */
  display: number;
}

/** A discrete input event worth showing a flourish for. */
export interface PulseEvent {
  kind: "click" | "key";
  display: number;
}

export type ControlPhase = "idle" | "active";

export interface ControlState {
  phase: ControlPhase;
  /** Which agent is driving, e.g. "claude code". Null when idle. */
  agent: string | null;
  /** Live mode for this session; starts from `defaultAccess`. */
  mode: AccessMode;
  /** Human-readable current action, e.g. "clicking". */
  action: string | null;
  /**
   * A panic stop is latched. Every tool stays refused until the user hands
   * control back — the stop button is not meant to be undone by an agent that
   * simply retries.
   */
  stopped: boolean;
}

export interface PendingApproval {
  id: string;
  tool: string;
  summary: string;
  detail: string | null;
  agent: string | null;
}

export interface AgentTarget {
  id: string;
  name: string;
  /** Absolute path of the config file we would write. */
  configPath: string;
  /** Whether that agent appears to be installed on this machine. */
  detected: boolean;
  /** Whether a conduit entry is already present. */
  installed: boolean;
  /** Populated when detection or install hit a problem. */
  error: string | null;
  /** data: URL of the vendor app's icon, when that app is installed. */
  icon: string | null;
}
