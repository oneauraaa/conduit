import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as standalone from "./standalone";
import type {
  AccessMode,
  AgentTarget,
  ControlState,
  CursorEvent,
  PendingApproval,
  Readiness,
  PulseEvent,
  ServerState,
  Settings,
  ToolCallEvent,
  TailscaleState,
  ToolDef,
  ToolsAccess,
} from "./types";

/**
 * Outside the Tauri shell there is no Rust core to answer, so commands resolve
 * against the sample data in `standalone.ts` instead. This keeps the whole UI
 * reviewable in a plain browser; in the packaged app the branch never runs.
 */
const local: Record<string, (args: Record<string, unknown>) => unknown> = {
  get_server_state: () => standalone.server,
  start_server: () => ({ ...standalone.server, status: "running" }),
  stop_server: () => ({ ...standalone.server, status: "stopped", startedAt: null }),
  restart_server: () => standalone.server,
  set_port: (a) => ({ ...standalone.server, port: a.port }),
  get_settings: () => standalone.settings,
  get_tool_catalog: () => standalone.catalog,
  set_default_access: (a) => ({ ...standalone.settings, defaultAccess: a.mode }),
  set_tools_access: (a) => ({ ...standalone.settings, toolsAccess: a.access }),
  set_tool_enabled: () => standalone.settings,
  // These two mutate the sample object rather than returning a one-off spread.
  // The browser access panel is the one place in the UI whose state
  // accumulates — switch it on, then add an address — and a stateless stub
  // would silently undo the switch on the next call, which makes the panel
  // impossible to review with `pnpm dev`.
  set_cors_enabled: (a) => {
    standalone.settings.corsEnabled = a.enabled as boolean;
    return { ...standalone.settings };
  },
  set_cors_origins: (a) => {
    standalone.settings.corsOrigins = a.origins as string[];
    return { ...standalone.settings };
  },
  set_start_on_login: (a) => {
    standalone.settings.startOnLogin = a.enabled as boolean;
    return { ...standalone.settings };
  },
  set_start_hidden: (a) => {
    standalone.settings.startHidden = a.hidden as boolean;
    return { ...standalone.settings };
  },
  get_readiness: () => standalone.readiness,
  get_control_state: () => standalone.control,
  set_session_mode: (a) => ({ ...standalone.control, mode: a.mode }),
  resume_control: () => ({ ...standalone.control, stopped: false }),
  list_agents: () => standalone.agents,
  get_tailscale_state: () => standalone.tailscale,
  enable_remote: () => ({ ...standalone.tailscale, sharing: true }),
  disable_remote: () => ({ ...standalone.tailscale, sharing: false, publicUrl: null }),
  regenerate_remote_token: () => standalone.tailscale,
  install_agent: (a) => ({
    ...standalone.agents.find((x) => x.id === a.id)!,
    installed: true,
  }),
  uninstall_agent: (a) => ({
    ...standalone.agents.find((x) => x.id === a.id)!,
    installed: false,
  }),
};

function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  if (standalone.isStandalone) {
    const handler = local[cmd];
    return Promise.resolve((handler ? handler(args) : undefined) as T);
  }
  return tauriInvoke<T>(cmd, args);
}

/* ── server ─────────────────────────────────────────────────── */

export const getServerState = () => invoke<ServerState>("get_server_state");
export const startServer = () => invoke<ServerState>("start_server");
export const stopServer = () => invoke<ServerState>("stop_server");
export const restartServer = () => invoke<ServerState>("restart_server");
export const setPort = (port: number) => invoke<ServerState>("set_port", { port });

/* ── settings & tools ───────────────────────────────────────── */

export const getSettings = () => invoke<Settings>("get_settings");
export const getToolCatalog = () => invoke<ToolDef[]>("get_tool_catalog");
export const setDefaultAccess = (mode: AccessMode) =>
  invoke<Settings>("set_default_access", { mode });
export const setToolsAccess = (access: ToolsAccess) =>
  invoke<Settings>("set_tools_access", { access });
export const setToolEnabled = (tool: string, enabled: boolean) =>
  invoke<Settings>("set_tool_enabled", { tool, enabled });

/* ── browser access ─────────────────────────────────────────── */

export const setCorsEnabled = (enabled: boolean) =>
  invoke<Settings>("set_cors_enabled", { enabled });
export const setCorsOrigins = (origins: string[]) =>
  invoke<Settings>("set_cors_origins", { origins });

/* ── startup ────────────────────────────────────────────────── */

/** Rejects if the OS refused the login entry, so the switch can't lie. */
export const setStartOnLogin = (enabled: boolean) =>
  invoke<Settings>("set_start_on_login", { enabled });
export const setStartHidden = (hidden: boolean) =>
  invoke<Settings>("set_start_hidden", { hidden });

/* ── readiness ──────────────────────────────────────────────── */

export const getReadiness = () => invoke<Readiness>("get_readiness");
export const requestAccessibility = () => invoke<void>("request_accessibility");
export const requestScreenRecording = () => invoke<void>("request_screen_recording");
/** macOS only; a no-op on Windows, which withholds nothing behind a pane. */
export const openPermissionSettings = (which: "accessibility" | "screen") =>
  invoke<void>("open_permission_settings", { which });
/** Windows only. Tears down this process on success, so nothing after it runs. */
export const relaunchElevated = () => invoke<void>("relaunch_elevated");

/* ── control session ────────────────────────────────────────── */

export const getControlState = () => invoke<ControlState>("get_control_state");
export const setSessionMode = (mode: AccessMode) =>
  invoke<ControlState>("set_session_mode", { mode });
export const stopControl = () => invoke<void>("stop_control");
/** Clears a latched panic stop so agents may start a new session. */
export const resumeControl = () => invoke<ControlState>("resume_control");
export const resolveApproval = (id: string, decision: "allow" | "session" | "deny") =>
  invoke<void>("resolve_approval", { id, decision });

/* ── agents ─────────────────────────────────────────────────── */

export const listAgents = () => invoke<AgentTarget[]>("list_agents");
export const installAgent = (id: string) => invoke<AgentTarget>("install_agent", { id });
export const uninstallAgent = (id: string) => invoke<AgentTarget>("uninstall_agent", { id });

/* ── tailscale sharing ──────────────────────────────────────── */

export const getTailscaleState = () => invoke<TailscaleState>("get_tailscale_state");
export const enableRemote = () => invoke<TailscaleState>("enable_remote");
export const disableRemote = () => invoke<TailscaleState>("disable_remote");
export const regenerateRemoteToken = () => invoke<TailscaleState>("regenerate_remote_token");

/* ── window chrome ──────────────────────────────────────────── */

export const hideToTray = () => invoke<void>("hide_to_tray");

/* ── events ─────────────────────────────────────────────────── */

type EventMap = {
  "server:state": ServerState;
  "server:tool-call": ToolCallEvent;
  "settings:changed": Settings;
  "readiness:changed": Readiness;
  "control:state": ControlState;
  "control:cursor": CursorEvent;
  "control:pulse": PulseEvent;
  "control:approval": PendingApproval | null;
  "agents:changed": AgentTarget[];
  "tailscale:state": TailscaleState;
};

export function on<K extends keyof EventMap>(
  event: K,
  handler: (payload: EventMap[K]) => void,
): Promise<UnlistenFn> {
  if (standalone.isStandalone) return Promise.resolve(() => {});
  return listen<EventMap[K]>(event, (e) => handler(e.payload));
}

/**
 * Subscribe for the lifetime of an effect. Returns a cleanup that is safe to
 * call before the listener has finished registering — React 19 strict mode
 * mounts effects twice, and an un-awaited listen() would otherwise leak.
 */
export function subscribe<K extends keyof EventMap>(
  event: K,
  handler: (payload: EventMap[K]) => void,
): () => void {
  let unlisten: UnlistenFn | null = null;
  let cancelled = false;

  on(event, handler).then((fn) => {
    if (cancelled) fn();
    else unlisten = fn;
  });

  return () => {
    cancelled = true;
    unlisten?.();
  };
}
