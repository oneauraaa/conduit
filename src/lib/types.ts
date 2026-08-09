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
