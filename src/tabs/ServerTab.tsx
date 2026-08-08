import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  Accessibility,
  Check,
  Copy,
  MonitorPlay,
  Play,
  RotateCw,
  Square,
  TriangleAlert,
} from "lucide-react";
import { Button, Card, Row, SectionLabel, TabShell } from "@/components/Panel";
import { StatusDot } from "@/components/StatusDot";
import {
  getPermissions,
  openPermissionSettings,
  restartServer,
  setPort as setPortIpc,
  startServer,
  stopServer,
  subscribe,
} from "@/lib/ipc";
import type { ControlState, PermissionState, ServerState, ToolCallEvent } from "@/lib/types";
import { cn } from "@/lib/cn";

const MAX_LOG = 40;

function useUptime(startedAt: number | null) {
  const [, tick] = useState(0);
  useEffect(() => {
    if (startedAt === null) return;
    const t = setInterval(() => tick((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [startedAt]);

  if (startedAt === null) return null;
  const secs = Math.max(0, Math.floor((Date.now() - startedAt) / 1000));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  return h > 0 ? `${h}h ${m}m` : m > 0 ? `${m}m ${s}s` : `${s}s`;
}

export function ServerTab({ server }: { server: ServerState }) {
  const [perms, setPerms] = useState<PermissionState>({
    accessibility: false,
    screenRecording: false,
  });
  const [log, setLog] = useState<ToolCallEvent[]>([]);
  const [copied, setCopied] = useState(false);
  const [portDraft, setPortDraft] = useState(String(server.port));
  const [control, setControl] = useState<ControlState | null>(null);

  const uptime = useUptime(server.startedAt);
  const running = server.status === "running";
  const url = useMemo(() => `http://127.0.0.1:${server.port}/mcp`, [server.port]);
  /** Name of the agent currently driving, if any. */
  const driving = control?.phase === "active" ? (control.agent ?? "an agent") : null;

  useEffect(() => setPortDraft(String(server.port)), [server.port]);

  useEffect(() => {
    void getPermissions().then(setPerms).catch(() => {});
    const offPerms = subscribe("permissions:changed", setPerms);
    const offLog = subscribe("server:tool-call", (e) =>
      setLog((prev) => [e, ...prev].slice(0, MAX_LOG)),
    );
    const offControl = subscribe("control:state", setControl);
    return () => {
      offPerms();
      offLog();
      offControl();
    };
  }, []);

  // macOS only re-evaluates TCC grants for a running process on the next check,
  // so poll while the window is visible to catch a grant made in System Settings.
  useEffect(() => {
    const t = setInterval(() => {
      if (!document.hidden) void getPermissions().then(setPerms).catch(() => {});
    }, 2000);
    return () => clearInterval(t);
  }, []);

  async function copyUrl() {
    await navigator.clipboard.writeText(url);
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  }

  function commitPort() {
    const n = Number(portDraft);
    if (!Number.isInteger(n) || n < 1024 || n > 65535) {
      setPortDraft(String(server.port));
      return;
    }
    if (n !== server.port) void setPortIpc(n).catch(() => setPortDraft(String(server.port)));
  }

  return (
    <TabShell>
      {/* ── status ─────────────────────────────────────────── */}
      <Card className="relative">
        {running && (
          <div className="pointer-events-none absolute inset-x-0 top-0 h-px bg-[linear-gradient(90deg,transparent,var(--color-aqua),transparent)]" />
        )}
        <div className="flex items-center gap-3 px-3.5 pt-3.5 pb-3">
          <StatusDot
            tone={
              server.status === "error"
                ? "error"
                : driving
                  ? "active"
                  : running
                    ? "live"
                    : "idle"
            }
            size={9}
          />
          <div className="min-w-0 flex-1">
            <div className="text-[13px] leading-tight font-semibold">
              {server.status === "running"
                ? "server running"
                : server.status === "starting"
                  ? "starting…"
                  : server.status === "error"
                    ? "server error"
                    : "server stopped"}
            </div>
            <div className="mt-0.5 truncate text-[11px] text-[rgb(var(--text-dim))]">
              {server.status === "error" && server.lastError
                ? server.lastError
                : running
                  ? driving
                    ? `${driving} is in control · up ${uptime}`
                    : `up ${uptime} · waiting`
                  : "agents cannot reach conduit while stopped"}
            </div>
          </div>

          <div className="flex items-center gap-1.5">
            {running ? (
              <>
                <Button onClick={() => void restartServer()} title="Restart the MCP server">
                  <RotateCw size={12} /> restart
                </Button>
                <Button onClick={() => void stopServer()} variant="danger" title="Stop the MCP server">
                  <Square size={11} /> stop
                </Button>
              </>
            ) : (
              <Button
                onClick={() => void startServer()}
                variant="primary"
                disabled={server.status === "starting"}
              >
                <Play size={12} /> start
              </Button>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2 border-t hairline bg-[rgb(var(--surface-sunken)/0.5)] px-3.5 py-2">
          <code className="selectable min-w-0 flex-1 truncate font-mono text-[11px] text-[rgb(var(--text-dim))]">
            {url}
          </code>
          <button
            type="button"
            onClick={() => void copyUrl()}
            title="Copy endpoint"
            className="grid size-6 place-items-center rounded-md text-[rgb(var(--text-faint))] transition-colors hover:bg-[rgb(var(--text-faint)/0.14)] hover:text-[rgb(var(--text))]"
          >
            <AnimatePresence mode="wait" initial={false}>
              <motion.span
                key={copied ? "ok" : "copy"}
                initial={{ opacity: 0, scale: 0.6 }}
                animate={{ opacity: 1, scale: 1 }}
                exit={{ opacity: 0, scale: 0.6 }}
                transition={{ duration: 0.15 }}
              >
                {copied ? (
                  <Check size={12} className="text-[var(--color-aqua)]" />
                ) : (
                  <Copy size={12} />
                )}
              </motion.span>
            </AnimatePresence>
          </button>
          <div className="h-3.5 w-px bg-[rgb(var(--border)/var(--border-alpha))]" />
          <label className="flex items-center gap-1.5 text-[11px] text-[rgb(var(--text-faint))]">
            port
            <input
              value={portDraft}
              onChange={(e) => setPortDraft(e.target.value.replace(/\D/g, "").slice(0, 5))}
              onBlur={commitPort}
              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              inputMode="numeric"
              className="w-12 rounded border hairline bg-[rgb(var(--surface))] px-1.5 py-0.5 text-center font-mono text-[11px] tabular-nums text-[rgb(var(--text))] outline-none focus:border-[rgb(var(--accent))]"
            />
          </label>
        </div>
      </Card>

      {/* ── permissions ────────────────────────────────────── */}
      <div className="flex flex-col gap-2">
        <SectionLabel>macos permissions</SectionLabel>
        <Card>
          <PermissionRow
            icon={<Accessibility size={15} />}
            title="accessibility"
            granted={perms.accessibility}
            need="needed to move the cursor, click, type and read the screen"
            onGrant={() => void openPermissionSettings("accessibility")}
          />
          <PermissionRow
            icon={<MonitorPlay size={15} />}
            title="screen recording"
            granted={perms.screenRecording}
            need="needed for screenshots, so the agent can see what it's doing"
            onGrant={() => void openPermissionSettings("screen")}
          />
        </Card>
      </div>

      {/* ── activity ───────────────────────────────────────── */}
      <div className="flex flex-col gap-2">
        <SectionLabel>activity</SectionLabel>
        <Card tone="sunken">
          {log.length === 0 ? (
            <div className="grid min-h-[64px] place-items-center px-6 py-4 text-center">
              <p className="text-[11px] leading-relaxed text-[rgb(var(--text-faint))]">
                {running
                  ? "waiting for an agent to connect"
                  : "start the server to accept agent connections"}
              </p>
            </div>
          ) : (
            <div className="max-h-[168px] overflow-y-auto">
              <AnimatePresence initial={false}>
                {log.map((e) => (
                  <motion.div
                    key={e.id}
                    layout
                    initial={{ opacity: 0, y: -6 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
                    className="flex items-center gap-2.5 border-b hairline px-3.5 py-1.5 last:border-b-0"
                  >
                    <StatusDot
                      tone={
                        e.outcome === "ok" ? "live" : e.outcome === "error" ? "error" : "active"
                      }
                      size={5}
                    />
                    <span className="font-mono text-[11px] font-medium">{e.tool}</span>
                    {e.detail && (
                      <span className="min-w-0 flex-1 truncate text-[10.5px] text-[rgb(var(--text-faint))]">
                        {e.detail}
                      </span>
                    )}
                    <span
                      className={cn(
                        "ml-auto shrink-0 text-[10px] tabular-nums",
                        e.outcome === "ok"
                          ? "text-[rgb(var(--text-faint))]"
                          : "text-amber-600 dark:text-amber-400",
                      )}
                    >
                      {e.outcome === "ok"
                        ? e.durationMs !== null
                          ? `${e.durationMs}ms`
                          : ""
                        : e.outcome}
                    </span>
                  </motion.div>
                ))}
              </AnimatePresence>
            </div>
          )}
        </Card>
      </div>
    </TabShell>
  );
}

function PermissionRow({
  icon,
  title,
  granted,
  need,
  onGrant,
}: {
  icon: React.ReactNode;
  title: string;
  granted: boolean;
  need: string;
  onGrant: () => void;
}) {
  return (
    <Row
      icon={
        <span className={granted ? "text-[var(--color-aqua)]" : "text-amber-500"}>{icon}</span>
      }
      title={title}
      description={granted ? "granted" : need}
    >
      {granted ? (
        <span className="flex items-center gap-1 text-[11px] font-medium text-[var(--color-aqua)]">
          <Check size={12} strokeWidth={3} />
        </span>
      ) : (
        <Button onClick={onGrant}>
          <TriangleAlert size={11} /> grant
        </Button>
      )}
    </Row>
  );
}
