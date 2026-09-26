import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as standalone from "./standalone";
import type {
  AccessMode,
  AgentTarget,
  BrowserPermissionCategory,
  BrowserPermissionMode,
  BrowserMode,
  BrowserRestartStrategy,
  BrowserState,
  ControlState,
  CursorEvent,
  HyprlandState,
  NewSandbox,
  PendingApproval,
  Readiness,
  PulseEvent,
  SandboxActivity,
  SandboxPatch,
  SandboxesState,
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
  get_tool_catalog: () =>
    standalone.browser.install.status === "ready"
      ? [...standalone.catalog, ...standalone.browserCatalog]
      : standalone.catalog,
  set_default_access: (a) => ({ ...standalone.settings, defaultAccess: a.mode }),
  set_tools_access: (a) => {
    const access = a.access as ToolsAccess;
    if (access === "all" || (standalone.settings.toolsAccess === "all" && access === "custom")) {
      standalone.settings.toolToggles = {};
    }
    standalone.settings.toolsAccess = access;
    return { ...standalone.settings };
  },
  set_tool_enabled: (a) => {
    const tool = a.tool as string;
    const enabled = a.enabled as boolean;
    if (standalone.settings.toolsAccess === "all" && !enabled) {
      standalone.settings.toolToggles = { [tool]: false };
      standalone.settings.toolsAccess = "custom";
    } else if (standalone.settings.toolsAccess === "custom") {
      standalone.settings.toolToggles[tool] = enabled;
      if ([...standalone.catalog, ...standalone.browserCatalog].every(
        (entry) => standalone.settings.toolToggles[entry.name] ?? true,
      )) {
        standalone.settings.toolToggles = {};
        standalone.settings.toolsAccess = "all";
      }
    }
    return { ...standalone.settings };
  },
  get_browser_state: () => ({ ...standalone.browser }),
  refresh_browser_install: () => ({ ...standalone.browser }),
  install_browser: () => {
    standalone.browser.install.status = "ready";
    standalone.browser.install.installedRevision = standalone.browser.install.expectedRevision;
    standalone.browser.install.downloadedBytes = standalone.browser.install.totalBytes ?? 0;
    standalone.browser.install.error = null;
    if (standalone.settings.browserAutoStart) standalone.browser.runStatus = "running";
    return { ...standalone.browser };
  },
  cancel_browser_install: () => undefined,
  uninstall_browser: () => {
    standalone.browser.install.status = "unavailable";
    standalone.browser.install.installedRevision = null;
    standalone.browser.runStatus = "stopped";
    standalone.browser.stopLatched = true;
    standalone.browser.tabs = [];
    return { ...standalone.browser };
  },
  start_browser: () => {
    standalone.browser.runStatus = "running";
    standalone.browser.stopLatched = false;
    return { ...standalone.browser };
  },
  stop_browser: () => {
    standalone.browser.runStatus = "stopped";
    standalone.browser.stopLatched = true;
    standalone.browser.tabs = [];
    return { ...standalone.browser };
  },
  set_browser_mode: (a) => {
    standalone.browser.mode = a.mode as BrowserMode;
    return { ...standalone.browser };
  },
  select_browser_profile: (a) => {
    standalone.browser.selectedProfileId = a.profileId as string;
    return { ...standalone.browser };
  },
  create_browser_profile: (a) => {
    const profile = { id: `profile-${Date.now()}`, name: a.name as string, incognito: false };
    standalone.browser.profiles.splice(-1, 0, profile);
    return { ...standalone.browser };
  },
  rename_browser_profile: (a) => {
    const profile = standalone.browser.profiles.find((item) => item.id === a.id);
    if (profile) profile.name = a.name as string;
    return { ...standalone.browser };
  },
  delete_browser_profile: (a) => {
    standalone.browser.profiles = standalone.browser.profiles.filter((item) => item.id !== a.id);
    return { ...standalone.browser };
  },
  set_browser_permission: (a) => {
    standalone.settings.browserPermissions[a.category as BrowserPermissionCategory] =
      a.mode as BrowserPermissionMode;
    return { ...standalone.settings };
  },
  set_browser_tab_visible: () => undefined,
  open_browser_downloads: () => undefined,
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
  set_browser_auto_start: (a) => {
    standalone.settings.browserAutoStart = a.enabled as boolean;
    return { ...standalone.settings };
  },
  set_outline_desktop: (a) => {
    standalone.settings.outlineDesktop = a.enabled as boolean;
    return { ...standalone.settings };
  },
  set_outline_browser: (a) => {
    standalone.settings.outlineBrowser = a.enabled as boolean;
    return { ...standalone.settings };
  },
  set_outline_background: (a) => {
    standalone.settings.outlineBackground = a.enabled as boolean;
    return { ...standalone.settings };
  },
  set_pill_desktop: (a) => {
    standalone.settings.pillDesktop = a.enabled as boolean;
    return { ...standalone.settings };
  },
  set_pill_browser: (a) => {
    standalone.settings.pillBrowser = a.enabled as boolean;
    return { ...standalone.settings };
  },
  set_pill_background: (a) => {
    standalone.settings.pillBackground = a.enabled as boolean;
    return { ...standalone.settings };
  },
  get_readiness: () => standalone.readiness,
  get_control_state: () => standalone.control,
  get_pending_approval: () => null,
  set_session_mode: (a) => ({ ...standalone.control, mode: a.mode }),
  resume_control: () => ({ ...standalone.control, stopped: false }),
  list_agents: () => standalone.agents,
  get_tailscale_state: () => standalone.tailscale,
  get_hyprland_state: () => standalone.hyprland,
  enable_remote: () => ({ ...standalone.tailscale, sharing: true }),
  enable_remote_with_password: () => ({ ...standalone.tailscale, sharing: true }),
  disable_remote: () => ({ ...standalone.tailscale, sharing: false, publicUrl: null }),
  regenerate_remote_token: () => standalone.tailscale,
  install_agent: (a) => {
    const agent = standalone.agents.find((x) => x.id === a.id)!;
    const target = (a.target as string | undefined) ?? "host";
    if (!agent.installedTargets.includes(target)) agent.installedTargets.push(target);
    agent.installed = agent.installedTargets.includes("host");
    return { ...agent };
  },
  uninstall_agent: (a) => {
    const agent = standalone.agents.find((x) => x.id === a.id)!;
    const target = (a.target as string | undefined) ?? "host";
    agent.installedTargets = agent.installedTargets.filter((t) => t !== target);
    agent.installed = agent.installedTargets.includes("host");
    return { ...agent };
  },
  get_sandbox_state: () => structuredClone(standalone.sandboxes),
  refresh_docker: () => structuredClone(standalone.sandboxes),
  create_sandbox: (a) => {
    const sandbox = a.sandbox as NewSandbox;
    const id = sandbox.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "sandbox";
    standalone.sandboxes.sandboxes.push({
      spec: { ...sandbox, id, createdAt: Date.now() },
      status: "starting",
      error: null,
      build: null,
      paused: false,
      endpoint: `http://127.0.0.1:6767/sandbox/${id}/mcp`,
      busyCalls: 0,
    });
    return structuredClone(standalone.sandboxes);
  },
  update_sandbox: (a) => {
    const view = standalone.sandboxes.sandboxes.find((x) => x.spec.id === a.id);
    if (view) Object.assign(view.spec, a.patch as SandboxPatch);
    return structuredClone(standalone.sandboxes);
  },
  delete_sandbox: (a) => {
    standalone.sandboxes.sandboxes = standalone.sandboxes.sandboxes.filter((x) => x.spec.id !== a.id);
    return structuredClone(standalone.sandboxes);
  },
  start_sandbox: (a) => {
    const view = standalone.sandboxes.sandboxes.find((x) => x.spec.id === a.id);
    if (view) view.status = "running";
    return structuredClone(standalone.sandboxes);
  },
  stop_sandbox: (a) => {
    const view = standalone.sandboxes.sandboxes.find((x) => x.spec.id === a.id);
    if (view) view.status = "stopped";
    return structuredClone(standalone.sandboxes);
  },
  cancel_sandbox_build: () => undefined,
  set_sandbox_stop_on_quit: (a) => {
    standalone.sandboxes.stopOnQuit = a.stop as boolean;
    return structuredClone(standalone.sandboxes);
  },
  interrupt_sandbox_agent: (a) => {
    const view = standalone.sandboxes.sandboxes.find((x) => x.spec.id === a.id);
    if (view) view.paused = true;
    return structuredClone(standalone.sandboxes);
  },
  resume_sandbox_agent: (a) => {
    const view = standalone.sandboxes.sandboxes.find((x) => x.spec.id === a.id);
    if (view) view.paused = false;
    return structuredClone(standalone.sandboxes);
  },
  set_sandbox_tab_visible: () => undefined,
  open_docker_download: () => undefined,
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

/* ── built-in Chromium browser ─────────────────────────────── */

export const getBrowserState = () => invoke<BrowserState>("get_browser_state");
export const refreshBrowserInstall = () => invoke<BrowserState>("refresh_browser_install");
export const installBrowser = () => invoke<BrowserState>("install_browser");
export const uninstallBrowser = () => invoke<BrowserState>("uninstall_browser");
export const cancelBrowserInstall = () => invoke<void>("cancel_browser_install");
export const startBrowser = () => invoke<BrowserState>("start_browser");
export const stopBrowser = () => invoke<BrowserState>("stop_browser");
export const setBrowserMode = (mode: BrowserMode, restart?: BrowserRestartStrategy) =>
  invoke<BrowserState>("set_browser_mode", { mode, restart });
export const selectBrowserProfile = (
  profileId: string,
  restart?: BrowserRestartStrategy,
) => invoke<BrowserState>("select_browser_profile", { profileId, restart });
export const createBrowserProfile = (name: string) =>
  invoke<BrowserState>("create_browser_profile", { name });
export const renameBrowserProfile = (id: string, name: string) =>
  invoke<BrowserState>("rename_browser_profile", { id, name });
export const deleteBrowserProfile = (id: string) =>
  invoke<BrowserState>("delete_browser_profile", { id });
export const setBrowserPermission = (
  category: BrowserPermissionCategory,
  mode: BrowserPermissionMode,
) => invoke<Settings>("set_browser_permission", { category, mode });
export const setBrowserTabVisible = (visible: boolean) =>
  invoke<void>("set_browser_tab_visible", { visible });
export const openBrowserDownloads = () => invoke<void>("open_browser_downloads");

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
export const setBrowserAutoStart = (enabled: boolean) =>
  invoke<Settings>("set_browser_auto_start", { enabled });
export const setOutlineDesktop = (enabled: boolean) =>
  invoke<Settings>("set_outline_desktop", { enabled });
export const setOutlineBrowser = (enabled: boolean) =>
  invoke<Settings>("set_outline_browser", { enabled });
export const setOutlineBackground = (enabled: boolean) =>
  invoke<Settings>("set_outline_background", { enabled });
export const setPillDesktop = (enabled: boolean) =>
  invoke<Settings>("set_pill_desktop", { enabled });
export const setPillBrowser = (enabled: boolean) =>
  invoke<Settings>("set_pill_browser", { enabled });
export const setPillBackground = (enabled: boolean) =>
  invoke<Settings>("set_pill_background", { enabled });

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
export const getPendingApproval = () => invoke<PendingApproval | null>("get_pending_approval");
export const setSessionMode = (mode: AccessMode) =>
  invoke<ControlState>("set_session_mode", { mode });
export const stopControl = () => invoke<void>("stop_control");
/** Clears a latched panic stop so agents may start a new session. */
export const resumeControl = () => invoke<ControlState>("resume_control");
export const resolveApproval = (id: string, decision: "allow" | "session" | "deny") =>
  invoke<void>("resolve_approval", { id, decision });

/* ── agents ─────────────────────────────────────────────────── */

export const listAgents = () => invoke<AgentTarget[]>("list_agents");
/** `target` is "host" for this computer, else a sandbox id. */
export const installAgent = (id: string, target = "host") =>
  invoke<AgentTarget>("install_agent", { id, target });
export const uninstallAgent = (id: string, target = "host") =>
  invoke<AgentTarget>("uninstall_agent", { id, target });

/* ── sandboxes ──────────────────────────────────────────────── */

export const getSandboxState = () => invoke<SandboxesState>("get_sandbox_state");
export const refreshDocker = () => invoke<SandboxesState>("refresh_docker");
/** Creates and starts it; the first sandbox of an OS builds its image first. */
export const createSandbox = (sandbox: NewSandbox) =>
  invoke<SandboxesState>("create_sandbox", { sandbox });
export const updateSandbox = (id: string, patch: SandboxPatch) =>
  invoke<SandboxesState>("update_sandbox", { id, patch });
export const deleteSandbox = (id: string) => invoke<SandboxesState>("delete_sandbox", { id });
export const startSandbox = (id: string) => invoke<SandboxesState>("start_sandbox", { id });
export const stopSandbox = (id: string) => invoke<SandboxesState>("stop_sandbox", { id });
export const cancelSandboxBuild = (id: string) => invoke<void>("cancel_sandbox_build", { id });
export const setSandboxStopOnQuit = (stop: boolean) =>
  invoke<SandboxesState>("set_sandbox_stop_on_quit", { stop });
/** Cancels what the agent is doing in this sandbox and refuses its next calls. */
export const interruptSandboxAgent = (id: string) =>
  invoke<SandboxesState>("interrupt_sandbox_agent", { id });
export const resumeSandboxAgent = (id: string) =>
  invoke<SandboxesState>("resume_sandbox_agent", { id });
export const setSandboxTabVisible = (visible: boolean) =>
  invoke<void>("set_sandbox_tab_visible", { visible });
export const openDockerDownload = () => invoke<void>("open_docker_download");

/* ── tailscale sharing ──────────────────────────────────────── */

export const getTailscaleState = () => invoke<TailscaleState>("get_tailscale_state");
export const enableRemote = () => invoke<TailscaleState>("enable_remote");
export const enableRemoteWithPassword = (password: string) =>
  invoke<TailscaleState>("enable_remote_with_password", { password });
export const disableRemote = () => invoke<TailscaleState>("disable_remote");
export const regenerateRemoteToken = () => invoke<TailscaleState>("regenerate_remote_token");

/* ── hyprland ───────────────────────────────────────────────── */

/** Reports `available: false` off Hyprland rather than failing, so the tab has
 *  something to render on every platform. */
export const getHyprlandState = () => invoke<HyprlandState>("get_hyprland_state");

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
  "browser:state": BrowserState;
  "sandbox:state": SandboxesState;
  "sandbox:activity": SandboxActivity;
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
