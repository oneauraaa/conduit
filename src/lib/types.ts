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

export interface PermissionState {
  accessibility: boolean;
  screenRecording: boolean;
}

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
