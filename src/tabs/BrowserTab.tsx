import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  Download,
  Eye,
  EyeOff,
  FileDown,
  FileUp,
  FolderOpen,
  Globe2,
  History,
  LoaderCircle,
  PanelsTopLeft,
  Pencil,
  Plus,
  Power,
  ShieldCheck,
  Square,
  Trash2,
  TriangleAlert,
  UserRound,
} from "lucide-react";
import { Button, Card, Row, SectionLabel, TabShell } from "@/components/Panel";
import { Segmented, type SegmentOption } from "@/components/Segmented";
import { StatusDot } from "@/components/StatusDot";
import { cn } from "@/lib/cn";
import {
  cancelBrowserInstall,
  createBrowserProfile,
  deleteBrowserProfile,
  getBrowserState,
  getSettings,
  installBrowser,
  openBrowserDownloads,
  refreshBrowserInstall,
  renameBrowserProfile,
  selectBrowserProfile,
  setBrowserMode,
  setBrowserPermission,
  setBrowserTabVisible,
  startBrowser,
  stopBrowser,
  subscribe,
} from "@/lib/ipc";
import type {
  BrowserMode,
  BrowserPermissionCategory,
  BrowserPermissionMode,
  BrowserRestartStrategy,
  BrowserState,
  Settings,
} from "@/lib/types";

const MODES: readonly SegmentOption<BrowserMode>[] = [
  { value: "headless", label: "headless", hint: "Run quietly and show a live preview here" },
  { value: "visible", label: "visible", hint: "Open a separate Chromium window" },
];

const PERMISSIONS: readonly SegmentOption<BrowserPermissionMode>[] = [
  { value: "alwaysAllow", label: "always allow" },
  { value: "alwaysAsk", label: "always ask" },
];

const permissionRows: {
  category: BrowserPermissionCategory;
  title: string;
  description: string;
  icon: typeof Globe2;
}[] = [
  {
    category: "openWebsites",
    title: "open websites",
    description: "navigations, back, popups, and new nonblank tabs",
    icon: Globe2,
  },
  {
    category: "readHistory",
    title: "read navigation history",
    description: "search the history recorded by Conduit for the active profile",
    icon: History,
  },
  {
    category: "downloadFiles",
    title: "download files",
    description: "save a website download into this profile's Downloads folder",
    icon: FileDown,
  },
  {
    category: "uploadFiles",
    title: "upload files",
    description: "read a local file and send it to the active website",
    icon: FileUp,
  },
];

type PendingChange =
  | { kind: "mode"; value: BrowserMode }
  | { kind: "profile"; value: string };

function formatBytes(value: number | null): string {
  if (value === null) return "size from platform release manifest";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB"];
  let amount = value / 1024;
  let unit = units[0];
  for (let index = 1; amount >= 1024 && index < units.length; index += 1) {
    amount /= 1024;
    unit = units[index];
  }
  return `${amount.toFixed(amount >= 10 ? 0 : 1)} ${unit}`;
}

function DownloadPanel({ browser }: { browser: BrowserState }) {
  const [busy, setBusy] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const install = browser.install;
  const transferring = ["downloading", "verifying", "installing"].includes(install.status);
  const progress = install.totalBytes
    ? Math.min(100, (install.downloadedBytes / install.totalBytes) * 100)
    : 0;
  const phase =
    install.status === "downloading"
      ? "downloading Chromium"
      : install.status === "verifying"
        ? "verifying SHA-256"
        : install.status === "installing"
          ? "installing and testing"
          : "browser runtime required";

  async function download() {
    setBusy(true);
    setLocalError(null);
    try {
      await installBrowser();
    } catch (error) {
      setLocalError(String(error));
    } finally {
      setBusy(false);
    }
  }

  return (
    <TabShell>
      <div className="grid min-h-[580px] place-items-center">
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          className="w-full max-w-[520px] overflow-hidden rounded-2xl border hairline bg-[rgb(var(--surface-raised))] shadow-[0_18px_50px_-28px_rgb(27_107_255/0.65)]"
        >
          <div className="relative overflow-hidden px-7 pt-8 pb-7 text-center">
            <div className="absolute inset-x-20 -top-24 h-44 rounded-full bg-[rgb(var(--accent)/0.14)] blur-3xl" />
            <div className="conduit-gradient relative mx-auto grid size-14 place-items-center rounded-2xl text-white shadow-[0_10px_28px_-12px_rgb(27_107_255/0.8)]">
              <PanelsTopLeft size={29} strokeWidth={1.8} />
            </div>
            <h1 className="relative mt-4 text-[18px] font-semibold tracking-tight">add Chromium to Conduit</h1>
            <p className="relative mx-auto mt-1.5 max-w-[410px] text-[12px] leading-relaxed text-[rgb(var(--text-dim))]">
              Browser tools use a dedicated, sandboxed Chromium bundle. It is downloaded separately,
              so the main Conduit install stays small.
            </p>
          </div>

          <div className="border-t hairline bg-[rgb(var(--surface-sunken)/0.45)] px-6 py-5">
            <div className="flex items-center justify-between text-[11px]">
              <span className="font-medium text-[rgb(var(--text))]">{phase}</span>
              <span className="tabular-nums text-[rgb(var(--text-dim))]">
                {transferring
                  ? `${formatBytes(install.downloadedBytes)} / ${formatBytes(install.totalBytes)}`
                  : formatBytes(install.totalBytes)}
              </span>
            </div>

            <div className="mt-2.5 h-2 overflow-hidden rounded-full bg-[rgb(var(--text-faint)/0.18)]">
              <motion.div
                className={cn(
                  "h-full rounded-full conduit-gradient",
                  !install.totalBytes && transferring && "w-1/3 animate-breathe",
                )}
                animate={{ width: install.totalBytes ? `${progress}%` : transferring ? "34%" : "0%" }}
                transition={{ duration: 0.25, ease: "easeOut" }}
              />
            </div>

            {(install.error || localError) && (
              <div className="mt-3 flex items-start gap-2 rounded-lg border border-red-500/20 bg-red-500/8 px-3 py-2 text-left text-[11px] leading-snug text-red-600 dark:text-red-400">
                <TriangleAlert size={13} className="mt-px shrink-0" />
                <span className="selectable">{localError ?? install.error}</span>
              </div>
            )}

            <div className="mt-4 flex justify-center">
              {transferring || busy ? (
                <Button onClick={() => void cancelBrowserInstall()}>
                  <Square size={10} fill="currentColor" /> cancel
                </Button>
              ) : (
                <Button variant="primary" onClick={() => void download()}>
                  <Download size={13} /> {install.status === "error" ? "retry download" : "download Chromium"}
                </Button>
              )}
            </div>
            <p className="mt-3 text-center font-mono text-[9px] text-[rgb(var(--text-faint))]">
              {install.expectedRevision}
            </p>
          </div>
        </motion.div>
      </div>
    </TabShell>
  );
}

function RestartDialog({
  pending,
  onCancel,
  onChoose,
}: {
  pending: PendingChange;
  onCancel: () => void;
  onChoose: (strategy: BrowserRestartStrategy) => void;
}) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-50 grid place-items-center bg-[rgb(var(--surface-sunken)/0.72)] p-6 backdrop-blur-sm"
    >
      <motion.div
        initial={{ scale: 0.96, y: 6 }}
        animate={{ scale: 1, y: 0 }}
        exit={{ scale: 0.98, opacity: 0 }}
        className="w-full max-w-[430px] rounded-2xl border hairline bg-[rgb(var(--surface-raised))] p-5 app-shadow"
      >
        <div className="flex gap-3">
          <span className="grid size-9 shrink-0 place-items-center rounded-xl bg-amber-500/12 text-amber-500">
            <TriangleAlert size={18} />
          </span>
          <div>
            <h2 className="text-[15px] font-semibold">restart Chromium?</h2>
            <p className="mt-1 text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
              Changing the {pending.kind} restarts Chromium. Unsaved page state and in-progress forms may be lost.
            </p>
          </div>
        </div>
        <div className="mt-5 flex flex-wrap justify-end gap-2">
          <Button onClick={onCancel}>cancel</Button>
          <Button onClick={() => onChoose("fresh")}>restart fresh</Button>
          <Button variant="primary" onClick={() => onChoose("reopenUrls")}>restart and reopen URLs</Button>
        </div>
      </motion.div>
    </motion.div>
  );
}

export function BrowserTab() {
  const [browser, setBrowser] = useState<BrowserState | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [pending, setPending] = useState<PendingChange | null>(null);
  const [profileEditor, setProfileEditor] = useState<"create" | "rename" | null>(null);
  const [profileName, setProfileName] = useState("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void getBrowserState()
      .then((next) => {
        setBrowser(next);
        // An unavailable state can also mean an older installed revision. Its
        // saved manifest contains the old size, so always refresh metadata for
        // the bundle this Conduit build actually requires.
        if (next.install.status === "unavailable") {
          void refreshBrowserInstall().then(setBrowser).catch((reason) => setError(String(reason)));
        }
      })
      .catch((reason) => setError(String(reason)));
    void getSettings().then(setSettings).catch(() => {});
    void setBrowserTabVisible(true).catch(() => {});
    const offBrowser = subscribe("browser:state", setBrowser);
    const offSettings = subscribe("settings:changed", setSettings);
    return () => {
      offBrowser();
      offSettings();
      void setBrowserTabVisible(false).catch(() => {});
    };
  }, []);

  const selectedProfile = useMemo(
    () => browser?.profiles.find((profile) => profile.id === browser.selectedProfileId) ?? null,
    [browser],
  );

  if (!browser) return <TabShell />;
  if (browser.install.status !== "ready") return <DownloadPanel browser={browser} />;
  if (!settings) return <TabShell />;

  const running = browser.runStatus === "running";
  const canStop = running || browser.runStatus === "starting";
  const stopping = browser.runStatus === "stopping";
  const runtimeTransitioning = browser.runStatus === "starting" || stopping;
  const visibleError = error ?? (browser.runStatus === "crashed" ? browser.install.error : null);
  const persistentProfileCount = browser.profiles.filter((profile) => !profile.incognito).length;

  async function applyChange(change: PendingChange, restart?: BrowserRestartStrategy) {
    setError(null);
    try {
      const next =
        change.kind === "mode"
          ? await setBrowserMode(change.value, restart)
          : await selectBrowserProfile(change.value, restart);
      setBrowser(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setPending(null);
    }
  }

  function requestChange(change: PendingChange) {
    if (running) setPending(change);
    else void applyChange(change);
  }

  async function saveProfile() {
    const name = profileName.trim();
    if (!name) return;
    try {
      const next =
        profileEditor === "rename" && selectedProfile
          ? await renameBrowserProfile(selectedProfile.id, name)
          : await createBrowserProfile(name);
      setBrowser(next);
      setProfileEditor(null);
      setProfileName("");
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function removeProfile() {
    if (!selectedProfile || selectedProfile.incognito) return;
    if (!window.confirm(`Delete the “${selectedProfile.name}” browser profile, including its cookies and history?`)) return;
    try {
      setBrowser(await deleteBrowserProfile(selectedProfile.id));
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function toggleRuntime() {
    setError(null);
    try {
      setBrowser(canStop ? await stopBrowser() : await startBrowser());
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function updatePermission(
    category: BrowserPermissionCategory,
    mode: BrowserPermissionMode,
  ) {
    setError(null);
    try {
      setSettings(await setBrowserPermission(category, mode));
    } catch (reason) {
      setError(String(reason));
    }
  }

  return (
    <TabShell>
      <div className="flex items-center justify-between px-0.5">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-[17px] font-semibold tracking-tight">browser</h1>
            <span className="flex items-center gap-1.5 rounded-full border hairline bg-[rgb(var(--surface-raised))] px-2 py-0.5 text-[10px] text-[rgb(var(--text-dim))]">
              <StatusDot tone={running ? "live" : browser.runStatus === "crashed" ? "error" : "idle"} size={6} />
              {browser.runStatus}
            </span>
          </div>
          <p className="mt-0.5 text-[11px] text-[rgb(var(--text-dim))]">
            dedicated Chromium, controlled only through Conduit's browser tools
          </p>
        </div>
        <Button
          variant={canStop ? "danger" : "primary"}
          disabled={stopping}
          onClick={() => void toggleRuntime()}
        >
          {browser.runStatus === "starting" || stopping ? (
            <LoaderCircle size={12} className="animate-spin" />
          ) : canStop ? (
            <Square size={10} fill="currentColor" />
          ) : (
            <Power size={12} />
          )}
          {canStop ? "stop" : stopping ? "stopping" : "start"}
        </Button>
      </div>

      {visibleError && (
        <div className="flex items-start gap-2 rounded-xl border border-red-500/20 bg-red-500/8 px-3.5 py-2.5 text-[11px] text-red-600 dark:text-red-400">
          <TriangleAlert size={13} className="mt-px shrink-0" />
          <span className="selectable">{visibleError}</span>
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-[minmax(0,1.45fr)_minmax(300px,0.85fr)]">
        <div className="flex min-w-0 flex-col gap-4">
          <div className="overflow-hidden rounded-2xl border hairline bg-[rgb(var(--surface-sunken))] shadow-[inset_0_1px_rgb(255_255_255/0.03)]">
            <div className="flex items-center justify-between border-b hairline px-3.5 py-2">
              <div className="flex items-center gap-2 text-[11px] font-medium">
                {browser.mode === "headless" ? <Eye size={13} /> : <PanelsTopLeft size={13} />}
                {browser.mode === "headless" ? "live preview" : "visible Chromium"}
              </div>
              {browser.owner && (
                <span className="text-[9px] text-[rgb(var(--text-faint))]">controlled by {browser.owner}</span>
              )}
            </div>
            <div className="relative grid aspect-video place-items-center overflow-hidden bg-[linear-gradient(145deg,rgb(var(--surface-sunken)),rgb(4_18_46/0.92))]">
              {browser.mode === "headless" && browser.preview ? (
                <img
                  src={`data:${browser.preview.mimeType};base64,${browser.preview.data}`}
                  alt="View-only live preview of the agent's active Chromium page"
                  draggable={false}
                  className="size-full object-contain"
                />
              ) : (
                <div className="flex max-w-[300px] flex-col items-center px-6 text-center">
                  {browser.mode === "visible" ? (
                    <Eye size={27} className="text-[rgb(var(--accent-soft))]" />
                  ) : (
                    <EyeOff size={27} className="text-[rgb(var(--text-faint))]" />
                  )}
                  <p className="mt-2 text-[12px] font-medium text-white/85">
                    {!running
                      ? "Chromium is stopped"
                      : browser.mode === "visible"
                        ? "the page is open in a separate Chromium window"
                        : "waiting for the agent's first page"}
                  </p>
                  <p className="mt-1 text-[10px] leading-relaxed text-white/45">
                    {browser.mode === "headless"
                      ? "The preview is view-only and streams only while this tab is open."
                      : "Conduit shows tab and download status here without duplicating the window."}
                  </p>
                </div>
              )}
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <div className="flex min-w-0 flex-col gap-2">
              <SectionLabel>tabs</SectionLabel>
              <Card className="min-h-[86px]">
                {browser.tabs.length ? (
                  browser.tabs.map((tab) => (
                    <Row
                      key={`${tab.index}-${tab.url}`}
                      compact
                      icon={<Globe2 size={12} className={tab.active ? "text-[rgb(var(--accent))]" : undefined} />}
                      title={<span className="block truncate text-[11px]">{tab.title || "untitled page"}</span>}
                      description={<span className="block max-w-[240px] truncate font-mono text-[9px]">{tab.url}</span>}
                    >
                      {tab.active && <span className="rounded bg-[rgb(var(--accent)/0.1)] px-1.5 py-0.5 text-[8px] text-[rgb(var(--accent))]">agent</span>}
                    </Row>
                  ))
                ) : (
                  <p className="px-3.5 py-4 text-[11px] text-[rgb(var(--text-faint))]">no open pages</p>
                )}
              </Card>
            </div>

            <div className="flex min-w-0 flex-col gap-2">
              <div className="flex items-center justify-between">
                <SectionLabel>downloads</SectionLabel>
                <button
                  type="button"
                  onClick={() => void openBrowserDownloads()}
                  className="flex items-center gap-1 text-[9px] text-[rgb(var(--text-faint))] hover:text-[rgb(var(--text))]"
                >
                  <FolderOpen size={10} /> open folder
                </button>
              </div>
              <Card className="min-h-[86px]">
                {browser.downloads.length ? (
                  browser.downloads.slice(0, 4).map((download) => (
                    <Row
                      key={download.id}
                      compact
                      icon={<Download size={12} />}
                      title={<span className="block truncate text-[11px]">{download.filename}</span>}
                      description={download.error ?? download.status}
                    />
                  ))
                ) : (
                  <p className="px-3.5 py-4 text-[11px] text-[rgb(var(--text-faint))]">no website downloads</p>
                )}
              </Card>
            </div>
          </div>
        </div>

        <div className="flex min-w-0 flex-col gap-4">
          <div className="flex flex-col gap-2">
            <SectionLabel>runtime</SectionLabel>
            <Card>
              <Row title="window mode" description="headless previews here; visible opens a window">
                <Segmented
                  value={browser.mode}
                  options={MODES}
                  onChange={(value) => requestChange({ kind: "mode", value })}
                  disabled={runtimeTransitioning}
                  size="sm"
                />
              </Row>
              <Row
                title="profile"
                description={selectedProfile?.incognito ? "deleted whenever Chromium stops" : "cookies and sessions persist"}
              >
                <select
                  aria-label="browser profile"
                  value={browser.selectedProfileId}
                  disabled={runtimeTransitioning}
                  onChange={(event) => requestChange({ kind: "profile", value: event.target.value })}
                  className="max-w-[130px] rounded-lg border hairline bg-[rgb(var(--surface))] px-2 py-1.5 text-[11px] outline-none"
                >
                  {browser.profiles.map((profile) => (
                    <option key={profile.id} value={profile.id}>{profile.name}</option>
                  ))}
                </select>
                <button
                  type="button"
                  aria-label="create browser profile"
                  disabled={runtimeTransitioning}
                  onClick={() => { setProfileEditor("create"); setProfileName(""); }}
                  className="grid size-7 place-items-center rounded-lg disabled:opacity-30 hover:bg-[rgb(var(--text-faint)/0.12)]"
                >
                  <Plus size={12} />
                </button>
                <button
                  type="button"
                  aria-label="rename browser profile"
                  disabled={runtimeTransitioning || !selectedProfile || selectedProfile.incognito}
                  onClick={() => { setProfileEditor("rename"); setProfileName(selectedProfile?.name ?? ""); }}
                  className="grid size-7 place-items-center rounded-lg disabled:opacity-30 hover:bg-[rgb(var(--text-faint)/0.12)]"
                >
                  <Pencil size={11} />
                </button>
                <button
                  type="button"
                  aria-label="delete browser profile"
                  disabled={runtimeTransitioning || !selectedProfile || selectedProfile.incognito || persistentProfileCount <= 1}
                  onClick={() => void removeProfile()}
                  className="grid size-7 place-items-center rounded-lg text-[rgb(var(--text-faint))] disabled:opacity-30 hover:bg-red-500/10 hover:text-red-500"
                >
                  <Trash2 size={11} />
                </button>
              </Row>
              <AnimatePresence initial={false}>
                {profileEditor && (
                  <motion.div
                    initial={{ height: 0, opacity: 0 }}
                    animate={{ height: "auto", opacity: 1 }}
                    exit={{ height: 0, opacity: 0 }}
                    className="overflow-hidden"
                  >
                    <div className="flex gap-2 border-t hairline px-3.5 py-2.5">
                      <UserRound size={13} className="mt-1.5 shrink-0 text-[rgb(var(--text-faint))]" />
                      <input
                        autoFocus
                        value={profileName}
                        onChange={(event) => setProfileName(event.target.value)}
                        onKeyDown={(event) => event.key === "Enter" && void saveProfile()}
                        maxLength={64}
                        className="min-w-0 flex-1 rounded-lg border hairline bg-[rgb(var(--surface))] px-2 py-1 text-[11px] outline-none focus:border-[rgb(var(--accent))]"
                        placeholder="profile name"
                      />
                      <Button onClick={() => void saveProfile()}>save</Button>
                      <Button variant="ghost" onClick={() => setProfileEditor(null)}>cancel</Button>
                    </div>
                  </motion.div>
                )}
              </AnimatePresence>
            </Card>
          </div>

          <div className="flex flex-col gap-2">
            <SectionLabel>
              <span className="flex items-center gap-1.5"><ShieldCheck size={11} /> permissions</span>
            </SectionLabel>
            <Card>
              {permissionRows.map(({ category, title, description, icon: Icon }) => (
                <Row key={category} compact icon={<Icon size={13} />} title={title} description={description}>
                  <Segmented
                    value={settings.browserPermissions[category]}
                    options={PERMISSIONS}
                    onChange={(mode) => void updatePermission(category, mode)}
                    size="sm"
                  />
                </Row>
              ))}
            </Card>
            <p className="px-1 text-[9.5px] leading-relaxed text-[rgb(var(--text-faint))]">
              These four choices override Manual, Auto, and Full access. Tool switches, URL restrictions,
              sandboxing, and Panic Stop still take priority.
            </p>
          </div>
        </div>
      </div>

      <AnimatePresence>
        {pending && (
          <RestartDialog
            pending={pending}
            onCancel={() => setPending(null)}
            onChoose={(strategy) => void applyChange(pending, strategy)}
          />
        )}
      </AnimatePresence>
    </TabShell>
  );
}
