import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  Boxes,
  Check,
  Copy,
  Hand,
  LoaderCircle,
  MousePointer2,
  Play,
  Plus,
  Power,
  RotateCw,
  Square,
  Trash2,
  TriangleAlert,
  Unplug,
} from "lucide-react";
import { Button, Card, Row, SectionLabel, TabShell } from "@/components/Panel";
import { Segmented, type SegmentOption } from "@/components/Segmented";
import { StatusDot, type DotTone } from "@/components/StatusDot";
import { Switch } from "@/components/Switch";
import { NoVncView } from "@/components/sandbox/NoVncView";
import { cn } from "@/lib/cn";
import {
  cancelSandboxBuild,
  createSandbox,
  deleteSandbox,
  getSandboxState,
  interruptSandboxAgent,
  openDockerDownload,
  refreshDocker,
  resumeSandboxAgent,
  setSandboxStopOnQuit,
  setSandboxTabVisible,
  startSandbox,
  stopSandbox,
  subscribe,
  updateSandbox,
} from "@/lib/ipc";
import type {
  DockerStatus,
  NewSandbox,
  SandboxActivity,
  SandboxesState,
  SandboxOs,
  SandboxPatch,
  SandboxStatus,
  SandboxView,
} from "@/lib/types";

export const OS_LABEL: Record<SandboxOs, string> = {
  "ubuntu-24.04": "ubuntu 24.04",
  "ubuntu-22.04": "ubuntu 22.04",
  "debian-12": "debian 12",
};

const OS_OPTIONS: readonly SegmentOption<SandboxOs>[] = [
  { value: "ubuntu-24.04", label: OS_LABEL["ubuntu-24.04"], hint: "Ubuntu 24.04 LTS" },
  { value: "ubuntu-22.04", label: OS_LABEL["ubuntu-22.04"], hint: "Ubuntu 22.04 LTS" },
  { value: "debian-12", label: OS_LABEL["debian-12"], hint: "Debian 12 (bookworm)" },
];

const MEMORY_MB = [2048, 4096, 8192] as const;
const CPU_COUNTS = [1, 2, 4] as const;
const SCREENS = ["1280x800", "1440x900", "1920x1080"] as const;

const STATUS: Record<SandboxStatus, { label: string; tone: DotTone }> = {
  needsImage: { label: "not built", tone: "idle" },
  building: { label: "building", tone: "active" },
  stopped: { label: "stopped", tone: "idle" },
  starting: { label: "starting", tone: "active" },
  running: { label: "running", tone: "live" },
  stopping: { label: "stopping", tone: "active" },
  error: { label: "error", tone: "error" },
};

/** The options that fit what Docker can hand out, never fewer than the smallest. */
function fitting<T extends number>(values: readonly T[], limit: number | null, unit = 1): T[] {
  if (!limit) return [...values];
  const fit = values.filter((v) => v * unit <= limit);
  return fit.length ? fit : [values[0]];
}

function gb(mb: number) {
  return `${mb / 1024} GB`;
}

function resources(spec: SandboxView["spec"]) {
  return `${gb(spec.memoryMb)} · ${spec.cpus} cpu · ${spec.width}×${spec.height}`;
}

/* ── docker missing ───────────────────────────────────────────── */

function DockerPanel({ docker, onRetry }: { docker: DockerStatus; onRetry: () => void }) {
  const title =
    docker.availability === "checking"
      ? "looking for Docker"
      : docker.availability === "missing"
        ? "sandboxes need Docker"
        : docker.availability === "notRunning"
          ? "Docker is not running"
          : docker.availability === "noPermission"
            ? "Docker won't let conduit in"
            : "this Docker can't run sandboxes";

  return (
    <TabShell>
      <div className="grid min-h-[560px] place-items-center">
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          className="w-full max-w-[520px] overflow-hidden rounded-2xl border hairline bg-[rgb(var(--surface-raised))] shadow-[0_18px_50px_-28px_rgb(27_107_255/0.65)]"
        >
          <div className="relative overflow-hidden px-7 pt-8 pb-6 text-center">
            <div className="absolute inset-x-20 -top-24 h-44 rounded-full bg-[rgb(var(--accent)/0.14)] blur-3xl" />
            <div className="conduit-gradient relative mx-auto grid size-14 place-items-center rounded-2xl text-white shadow-[0_10px_28px_-12px_rgb(27_107_255/0.8)]">
              <Boxes size={28} strokeWidth={1.8} />
            </div>
            <h1 className="relative mt-4 text-[18px] font-semibold tracking-tight">{title}</h1>
            <p className="relative mx-auto mt-1.5 max-w-[420px] text-[12px] leading-relaxed text-[rgb(var(--text-dim))]">
              A sandbox is a Linux desktop in a container. Point an agent at one and it works
              there instead of on this computer, so your screen, mouse and keyboard stay yours.
            </p>
          </div>
          <div className="border-t hairline bg-[rgb(var(--surface-sunken)/0.45)] px-6 py-5">
            {docker.hint && (
              <p className="selectable text-center text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
                {docker.hint}
              </p>
            )}
            <div className="mt-4 flex justify-center gap-2">
              {docker.availability === "checking" ? (
                <span className="flex items-center gap-2 text-[11px] text-[rgb(var(--text-dim))]">
                  <LoaderCircle size={13} className="animate-spin" /> checking
                </span>
              ) : (
                <>
                  {docker.availability === "missing" && (
                    <Button variant="primary" onClick={() => void openDockerDownload()}>
                      get Docker
                    </Button>
                  )}
                  <Button onClick={onRetry}>
                    <RotateCw size={12} /> check again
                  </Button>
                </>
              )}
            </div>
          </div>
        </motion.div>
      </div>
    </TabShell>
  );
}

/* ── create ───────────────────────────────────────────────────── */

function CreateSandboxDialog({
  docker,
  onCancel,
  onCreated,
}: {
  docker: DockerStatus;
  onCancel: () => void;
  onCreated: (state: SandboxesState, name: string) => void;
}) {
  const memoryOptions = fitting(MEMORY_MB, docker.memoryBytes, 1024 * 1024);
  const cpuOptions = fitting(CPU_COUNTS, docker.cpus);
  const [draft, setDraft] = useState<NewSandbox>({
    name: "",
    os: "ubuntu-24.04",
    memoryMb: memoryOptions.includes(4096) ? 4096 : memoryOptions[memoryOptions.length - 1],
    cpus: cpuOptions.includes(2) ? 2 : cpuOptions[cpuOptions.length - 1],
    width: 1280,
    height: 800,
    internet: true,
    autoStart: true,
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function create() {
    setBusy(true);
    setError(null);
    try {
      onCreated(await createSandbox({ ...draft, name: draft.name.trim() }), draft.name.trim());
    } catch (reason) {
      setError(String(reason));
      setBusy(false);
    }
  }

  const screen = `${draft.width}x${draft.height}`;

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="fixed inset-0 z-50 grid place-items-center bg-[rgb(var(--surface-sunken)/0.72)] p-6 backdrop-blur-sm"
    >
      <motion.div
        role="dialog"
        aria-label="new sandbox"
        initial={{ scale: 0.96, y: 6 }}
        animate={{ scale: 1, y: 0 }}
        exit={{ scale: 0.98, opacity: 0 }}
        className="w-full max-w-[520px] rounded-2xl border hairline bg-[rgb(var(--surface-raised))] p-5 app-shadow"
      >
        <h2 className="text-[15px] font-semibold">new sandbox</h2>
        <p className="mt-1 text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
          The first sandbox of each system builds its desktop image — a few minutes and about
          1.5 GB, once. After that, new sandboxes start in seconds.
        </p>

        <div className="mt-4 flex flex-col gap-3">
          <label className="flex flex-col gap-1">
            <span className="text-[11px] font-medium">name</span>
            <input
              autoFocus
              value={draft.name}
              maxLength={40}
              placeholder="e.g. work"
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              onKeyDown={(e) => e.key === "Enter" && draft.name.trim() && void create()}
              className="rounded-lg border hairline bg-[rgb(var(--surface-sunken))] px-2.5 py-1.5 text-[12px] outline-none focus:border-[rgb(var(--accent))]"
            />
          </label>
          <DialogRow label="system">
            <Segmented
              size="sm"
              value={draft.os}
              options={OS_OPTIONS}
              onChange={(os) => setDraft({ ...draft, os })}
            />
          </DialogRow>
          <DialogRow label="memory">
            <Segmented
              size="sm"
              value={String(draft.memoryMb)}
              options={memoryOptions.map((mb) => ({ value: String(mb), label: gb(mb) }))}
              onChange={(mb) => setDraft({ ...draft, memoryMb: Number(mb) })}
            />
          </DialogRow>
          <DialogRow label="cpus">
            <Segmented
              size="sm"
              value={String(draft.cpus)}
              options={cpuOptions.map((n) => ({ value: String(n), label: String(n) }))}
              onChange={(n) => setDraft({ ...draft, cpus: Number(n) })}
            />
          </DialogRow>
          <DialogRow label="screen">
            <Segmented
              size="sm"
              value={screen}
              options={SCREENS.map((s) => ({ value: s, label: s.replace("x", "×") }))}
              onChange={(s) => {
                const [width, height] = s.split("x").map(Number);
                setDraft({ ...draft, width, height });
              }}
            />
          </DialogRow>
          <DialogRow label="internet access">
            <Switch
              label="internet access"
              checked={draft.internet}
              onChange={(internet) => setDraft({ ...draft, internet })}
            />
          </DialogRow>
          <DialogRow label="start when an agent calls it">
            <Switch
              label="start when an agent calls it"
              checked={draft.autoStart}
              onChange={(autoStart) => setDraft({ ...draft, autoStart })}
            />
          </DialogRow>
        </div>

        {error && <ErrorLine text={error} />}

        <div className="mt-5 flex justify-end gap-2">
          <Button onClick={onCancel} disabled={busy}>
            cancel
          </Button>
          <Button variant="primary" onClick={() => void create()} disabled={busy || !draft.name.trim()}>
            {busy ? <LoaderCircle size={12} className="animate-spin" /> : <Plus size={12} />} create
            and start
          </Button>
        </div>
      </motion.div>
    </motion.div>
  );
}

function DialogRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-[11px] font-medium">{label}</span>
      {children}
    </div>
  );
}

function ErrorLine({ text }: { text: string }) {
  return (
    <div className="mt-3 flex items-start gap-2 rounded-lg border border-red-500/20 bg-red-500/8 px-3 py-2 text-[11px] leading-snug text-red-600 dark:text-red-400">
      <TriangleAlert size={13} className="mt-px shrink-0" />
      <span className="selectable">{text}</span>
    </div>
  );
}

/* ── list ─────────────────────────────────────────────────────── */

function SandboxList({
  sandboxes,
  selected,
  onSelect,
}: {
  sandboxes: SandboxView[];
  selected: string | null;
  onSelect: (id: string) => void;
}) {
  return (
    <div className="flex flex-col gap-1.5" role="listbox" aria-label="sandboxes">
      {sandboxes.map((view) => {
        const active = view.spec.id === selected;
        const status = STATUS[view.status];
        return (
          <button
            key={view.spec.id}
            type="button"
            role="option"
            aria-selected={active}
            onClick={() => onSelect(view.spec.id)}
            className={cn(
              "flex flex-col gap-0.5 rounded-xl border px-3 py-2.5 text-left transition-colors",
              active
                ? "border-[rgb(var(--accent)/0.5)] bg-[rgb(var(--accent)/0.08)]"
                : "hairline bg-[rgb(var(--surface-raised))] hover:bg-[rgb(var(--surface-sunken))]",
            )}
          >
            <span className="flex items-center gap-2">
              <StatusDot tone={status.tone} size={6} />
              <span className="truncate text-[12.5px] font-medium">{view.spec.name}</span>
              {view.busyCalls > 0 && (
                <LoaderCircle size={11} className="ml-auto shrink-0 animate-spin text-[rgb(var(--accent))]" />
              )}
            </span>
            <span className="pl-3.5 text-[10.5px] text-[rgb(var(--text-dim))]">
              {OS_LABEL[view.spec.os]} · {status.label}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/* ── detail ───────────────────────────────────────────────────── */

function BuildPanel({ view, onStart }: { view: SandboxView; onStart: () => void }) {
  const build = view.build;
  const progress =
    build?.step && build.totalSteps ? Math.min(100, (build.step / build.totalSteps) * 100) : null;
  return (
    <div className="grid size-full place-items-center p-6">
      <div className="w-full max-w-[380px] text-center">
        {build ? (
          <>
            <p className="text-[12px] font-medium text-white">
              building the {OS_LABEL[view.spec.os]} desktop
              {build.step && build.totalSteps ? ` · step ${build.step} of ${build.totalSteps}` : ""}
            </p>
            <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-white/15">
              <motion.div
                className={cn("h-full rounded-full conduit-gradient", progress === null && "animate-breathe")}
                animate={{ width: progress === null ? "30%" : `${progress}%` }}
                transition={{ duration: 0.3, ease: "easeOut" }}
              />
            </div>
            <p className="mt-2 truncate font-mono text-[9.5px] text-white/50">{build.lastLine}</p>
            <div className="mt-3">
              <Button onClick={() => void cancelSandboxBuild(view.spec.id)}>
                <Square size={9} fill="currentColor" /> cancel
              </Button>
            </div>
          </>
        ) : (
          <>
            <p className="text-[12px] font-medium text-white">the desktop image isn't built yet</p>
            <p className="mt-1 text-[11px] text-white/60">
              starting builds it — a few minutes, once per system
            </p>
            <div className="mt-3">
              <Button variant="primary" onClick={onStart}>
                <Play size={11} /> build and start
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

/** "claude code is clicking · 640, 400" while it runs; what it was, after. */
function describe(activity: SandboxActivity): string {
  const detail = activity.detail ? ` · ${activity.detail}` : "";
  switch (activity.outcome) {
    case "running":
      return `${activity.agent ? `${activity.agent} is ` : ""}${activity.action}${detail}`;
    case "ok":
      return `${activity.tool}${detail}`;
    case "error":
      return `${activity.tool} failed${detail}`;
    case "blocked":
      return `${activity.tool} was refused`;
  }
}

/** How long a finished action stays on the live view. */
const BADGE_MS = 5000;

function ActivityBadge({ activity, width, height }: { activity: SandboxActivity; width: number; height: number }) {
  const pulse = activity.x !== null && activity.y !== null && activity.outcome !== "blocked";
  return (
    <>
      {pulse && (
        <motion.span
          key={`pulse-${activity.at}`}
          initial={{ opacity: 0.9, scale: 0.4 }}
          animate={{ opacity: 0, scale: 1.8 }}
          transition={{ duration: 0.7, ease: "easeOut" }}
          className="pointer-events-none absolute size-8 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-[var(--color-aqua)]"
          style={{ left: `${(activity.x! / width) * 100}%`, top: `${(activity.y! / height) * 100}%` }}
        />
      )}
      <motion.div
        key={`badge-${activity.at}`}
        initial={{ opacity: 0, y: 4 }}
        animate={{ opacity: 1, y: 0 }}
        className="pointer-events-none absolute bottom-2.5 left-2.5 flex max-w-[70%] items-center gap-1.5 rounded-full bg-black/60 px-2.5 py-1 text-[10.5px] text-white backdrop-blur"
      >
        {activity.outcome === "running" ? (
          <LoaderCircle size={10} className="shrink-0 animate-spin" />
        ) : (
          <StatusDot tone={activity.outcome === "ok" ? "live" : activity.outcome === "error" ? "error" : "active"} size={5} />
        )}
        <span className="truncate">{describe(activity)}</span>
      </motion.div>
    </>
  );
}

function SandboxDetail({
  view,
  docker,
  activity,
  onState,
  onError,
  onConnectAgent,
}: {
  view: SandboxView;
  docker: DockerStatus;
  activity: SandboxActivity | undefined;
  onState: (state: SandboxesState) => void;
  onError: (error: string | null) => void;
  onConnectAgent: (sandboxId: string) => void;
}) {
  const [interactive, setInteractive] = useState(false);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const { spec, status } = view;
  const running = status === "running";
  const moving = status === "starting" || status === "stopping" || status === "building";
  const [, setTick] = useState(0);
  const recent = activity && (activity.outcome === "running" || Date.now() - activity.at < BADGE_MS);

  // Nothing else re-renders when a finished action's time runs out, so ask
  // for one render at that moment.
  useEffect(() => {
    if (!activity || activity.outcome === "running") return;
    const left = activity.at + BADGE_MS - Date.now();
    if (left <= 0) return;
    const timer = setTimeout(() => setTick((n) => n + 1), left + 20);
    return () => clearTimeout(timer);
  }, [activity]);

  async function act(run: () => Promise<SandboxesState>) {
    setBusy(true);
    onError(null);
    try {
      onState(await run());
    } catch (reason) {
      onError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const patch = (next: SandboxPatch) => void act(() => updateSandbox(spec.id, next));

  async function copyEndpoint() {
    await navigator.clipboard.writeText(view.endpoint);
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  }

  async function remove() {
    if (
      !window.confirm(
        `Delete the “${spec.name}” sandbox? Its desktop, its home folder and everything an agent saved in it are removed, and agents set up for it are disconnected.`,
      )
    )
      return;
    await act(() => deleteSandbox(spec.id));
  }

  const screen = `${spec.width}x${spec.height}`;
  const memoryOptions = fitting(MEMORY_MB, docker.memoryBytes, 1024 * 1024);
  const cpuOptions = fitting(CPU_COUNTS, docker.cpus);
  const badge = STATUS[status];

  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h2 className="truncate text-[15px] font-semibold tracking-tight">{spec.name}</h2>
            <span className="flex items-center gap-1.5 rounded-full border hairline bg-[rgb(var(--surface-raised))] px-2 py-0.5 text-[10px] text-[rgb(var(--text-dim))]">
              <StatusDot tone={badge.tone} size={6} />
              {badge.label}
            </span>
          </div>
          <p className="mt-0.5 text-[11px] text-[rgb(var(--text-dim))]">
            {OS_LABEL[spec.os]} · {resources(spec)}
          </p>
        </div>
        <div className="flex shrink-0 gap-1.5">
          {running &&
            (view.paused ? (
              <Button onClick={() => void act(() => resumeSandboxAgent(spec.id))} disabled={busy}>
                <Play size={11} /> resume agent
              </Button>
            ) : (
              <Button
                variant="danger"
                onClick={() => void act(() => interruptSandboxAgent(spec.id))}
                disabled={busy}
                title="Cancel what the agent is doing here and refuse its next calls"
              >
                <Hand size={12} /> stop agent
              </Button>
            ))}
          {running || status === "starting" ? (
            <Button onClick={() => void act(() => stopSandbox(spec.id))} disabled={busy || moving}>
              <Power size={12} /> stop
            </Button>
          ) : (
            <Button
              variant="primary"
              onClick={() => void act(() => startSandbox(spec.id))}
              disabled={busy || moving}
            >
              {moving ? <LoaderCircle size={12} className="animate-spin" /> : <Play size={11} />} start
            </Button>
          )}
        </div>
      </div>

      {view.paused && (
        <div className="flex items-center gap-2 rounded-lg border border-amber-500/25 bg-amber-500/8 px-3 py-2 text-[11px] text-amber-700 dark:text-amber-300">
          <Hand size={13} className="shrink-0" />
          the agent is stopped here. its calls are refused until you resume it.
        </div>
      )}
      {view.error && <ErrorLine text={view.error} />}

      <div
        className="relative w-full overflow-hidden rounded-xl border hairline bg-[#0b1220]"
        style={{ aspectRatio: `${spec.width} / ${spec.height}` }}
      >
        {status === "needsImage" || status === "building" ? (
          <BuildPanel view={view} onStart={() => void act(() => startSandbox(spec.id))} />
        ) : (
          <NoVncView sandboxId={spec.id} running={running} interactive={interactive} />
        )}
        {running && recent && activity && (
          <ActivityBadge activity={activity} width={spec.width} height={spec.height} />
        )}
        {running && (
          <button
            type="button"
            onClick={() => setInteractive((v) => !v)}
            aria-pressed={interactive}
            title={interactive ? "Your mouse and keyboard reach the sandbox" : "Watch only"}
            className={cn(
              "absolute top-2.5 right-2.5 flex items-center gap-1.5 rounded-full px-2.5 py-1 text-[10.5px] backdrop-blur transition-colors",
              interactive ? "bg-[rgb(var(--accent))] text-white" : "bg-black/55 text-white/85 hover:bg-black/70",
            )}
          >
            <MousePointer2 size={11} /> {interactive ? "you have control" : "take control"}
          </button>
        )}
      </div>

      <Card>
        <Row
          title="endpoint"
          description={<span className="selectable font-mono text-[10.5px]">{view.endpoint}</span>}
        >
          <Button variant="ghost" onClick={() => void copyEndpoint()} title="Copy endpoint">
            {copied ? <Check size={12} className="text-[var(--color-aqua)]" /> : <Copy size={12} />}
          </Button>
          <Button onClick={() => onConnectAgent(spec.id)}>
            <Unplug size={12} /> connect an agent
          </Button>
        </Row>
      </Card>

      <div className="flex flex-col gap-2">
        <SectionLabel>settings</SectionLabel>
        <Card>
          <Row title="memory">
            <Segmented
              size="sm"
              value={String(spec.memoryMb)}
              options={memoryOptions.map((mb) => ({ value: String(mb), label: gb(mb) }))}
              onChange={(mb) => patch({ memoryMb: Number(mb) })}
              disabled={busy}
            />
          </Row>
          <Row title="cpus">
            <Segmented
              size="sm"
              value={String(spec.cpus)}
              options={cpuOptions.map((n) => ({ value: String(n), label: String(n) }))}
              onChange={(n) => patch({ cpus: Number(n) })}
              disabled={busy}
            />
          </Row>
          <Row title="screen" description={running ? "resizes the running desktop" : undefined}>
            <Segmented
              size="sm"
              value={screen}
              options={SCREENS.map((s) => ({ value: s, label: s.replace("x", "×") }))}
              onChange={(s) => {
                const [width, height] = s.split("x").map(Number);
                patch({ width, height });
              }}
              disabled={busy}
            />
          </Row>
          <Row
            title="internet access"
            description="off means no network at all. either way, the sandbox can never reach this computer."
          >
            <Switch
              label="internet access"
              checked={spec.internet}
              onChange={(internet) => patch({ internet })}
              disabled={busy}
            />
          </Row>
          <Row
            title="start when an agent calls it"
            description="otherwise an agent's calls fail until you start it here"
          >
            <Switch
              label="start when an agent calls it"
              checked={spec.autoStart}
              onChange={(autoStart) => patch({ autoStart })}
              disabled={busy}
            />
          </Row>
          <Row title="system" description="fixed once created">
            <span className="text-[11px] text-[rgb(var(--text-dim))]">{OS_LABEL[spec.os]}</span>
          </Row>
        </Card>
        <p className="px-0.5 text-[10.5px] leading-relaxed text-[rgb(var(--text-faint))]">
          agents act in a sandbox without asking — it's theirs to use. the tools tab's switches
          and panic stop still apply.
        </p>
      </div>

      <div className="flex justify-end">
        <Button variant="danger" onClick={() => void remove()} disabled={busy}>
          <Trash2 size={12} /> delete sandbox
        </Button>
      </div>
    </div>
  );
}

/* ── tab ──────────────────────────────────────────────────────── */

export function SandboxTab({ onConnectAgent }: { onConnectAgent: (sandboxId: string) => void }) {
  const [state, setState] = useState<SandboxesState | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activity, setActivity] = useState<Record<string, SandboxActivity>>({});

  useEffect(() => {
    void getSandboxState()
      .then((next) => {
        setState(next);
        if (next.docker.availability === "checking") void refreshDocker().then(setState);
      })
      .catch((reason) => setError(String(reason)));
    void setSandboxTabVisible(true).catch(() => {});
    const offState = subscribe("sandbox:state", setState);
    const offActivity = subscribe("sandbox:activity", (a) =>
      setActivity((prev) => ({ ...prev, [a.sandboxId]: a })),
    );
    return () => {
      offState();
      offActivity();
      void setSandboxTabVisible(false).catch(() => {});
    };
  }, []);

  // While Docker is missing or asleep, look again every few seconds, so
  // opening Docker Desktop is all the user has to do.
  const availability = state?.docker.availability;
  useEffect(() => {
    if (!availability || availability === "ready") return;
    const timer = setInterval(() => void refreshDocker().then(setState).catch(() => {}), 3000);
    return () => clearInterval(timer);
  }, [availability]);

  if (!state) return <TabShell />;
  if (state.docker.availability !== "ready") {
    return <DockerPanel docker={state.docker} onRetry={() => void refreshDocker().then(setState)} />;
  }

  const sandboxes = state.sandboxes;
  const current = sandboxes.find((s) => s.spec.id === selected) ?? sandboxes[0] ?? null;

  return (
    <TabShell>
      <div className="flex items-center justify-between px-0.5">
        <div>
          <h1 className="text-[17px] font-semibold tracking-tight">sandboxes</h1>
          <p className="mt-0.5 text-[11px] text-[rgb(var(--text-dim))]">
            linux desktops your agents use instead of this computer
          </p>
        </div>
        <Button variant="primary" onClick={() => setCreating(true)}>
          <Plus size={13} /> new sandbox
        </Button>
      </div>

      {error && <ErrorLine text={error} />}
      {state.docker.hint && (
        <p className="px-0.5 text-[10.5px] text-amber-700 dark:text-amber-300">{state.docker.hint}</p>
      )}

      {current ? (
        <div className="grid grid-cols-[200px_minmax(0,1fr)] items-start gap-4">
          <SandboxList sandboxes={sandboxes} selected={current.spec.id} onSelect={setSelected} />
          <SandboxDetail
            key={current.spec.id}
            view={current}
            docker={state.docker}
            activity={activity[current.spec.id]}
            onState={setState}
            onError={setError}
            onConnectAgent={onConnectAgent}
          />
        </div>
      ) : (
        <Card>
          <div className="flex flex-col items-center gap-3 px-6 py-10 text-center">
            <Boxes size={26} className="text-[rgb(var(--text-faint))]" />
            <div>
              <p className="text-[13px] font-medium">no sandboxes yet</p>
              <p className="mt-1 max-w-[380px] text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
                make one, then connect an agent to it from the agents tab. it gets its own desktop,
                apps and files — and never touches your screen.
              </p>
            </div>
            <Button variant="primary" onClick={() => setCreating(true)}>
              <Plus size={13} /> new sandbox
            </Button>
          </div>
        </Card>
      )}

      <Card>
        <Row
          title="stop sandboxes when conduit quits"
          description="so a forgotten desktop doesn't hold on to memory until you restart"
        >
          <Switch
            label="stop sandboxes when conduit quits"
            checked={state.stopOnQuit}
            onChange={(stop) =>
              void setSandboxStopOnQuit(stop).then(setState).catch((reason) => setError(String(reason)))
            }
          />
        </Row>
      </Card>

      <AnimatePresence>
        {creating && (
          <CreateSandboxDialog
            docker={state.docker}
            onCancel={() => setCreating(false)}
            onCreated={(next, name) => {
              setState(next);
              const made = next.sandboxes.find((s) => s.spec.name.toLowerCase() === name.toLowerCase());
              if (made) setSelected(made.spec.id);
              setCreating(false);
            }}
          />
        )}
      </AnimatePresence>
    </TabShell>
  );
}
