import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  Check,
  Copy,
  ExternalLink,
  Globe,
  Loader2,
  RefreshCw,
  ShieldAlert,
  TriangleAlert,
} from "lucide-react";
import { Button, Card, Row, SectionLabel, TabShell } from "@/components/Panel";
import { StatusDot } from "@/components/StatusDot";
import { Switch } from "@/components/Switch";
import {
  disableRemote,
  enableRemote,
  getTailscaleState,
  regenerateRemoteToken,
  subscribe,
} from "@/lib/ipc";
import { isWindows, machineName } from "@/lib/platform";
import { isStandalone } from "@/lib/standalone";
import type { TailscaleState } from "@/lib/types";

const DOWNLOAD = isWindows
  ? "https://tailscale.com/download/windows"
  : "https://tailscale.com/download/mac";

/**
 * Outside the Tauri shell, `?ts=missing` / `?ts=offline` / `?ts=idle` force the
 * states that are otherwise impossible to reach on a machine where Tailscale is
 * installed and running. Review-only; never reached in the packaged app.
 */
const demo = isStandalone
  ? new URLSearchParams(window.location.search).get("ts")
  : null;

export function TailscaleTab() {
  const [ts, setTs] = useState<TailscaleState | null>(null);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    void getTailscaleState()
      .then((s) => setTs(demo === "idle" ? { ...s, sharing: false, publicUrl: null } : s))
      .catch(() => {});
    return subscribe("tailscale:state", setTs);
  }, []);

  async function toggle(next: boolean) {
    setBusy(true);
    try {
      setTs(next ? await enableRemote() : await disableRemote());
    } catch (e) {
      setTs((prev) =>
        prev ? { ...prev, error: String(e), sharing: false } : prev,
      );
    } finally {
      setBusy(false);
    }
  }

  async function rotate() {
    setBusy(true);
    try {
      setTs(await regenerateRemoteToken());
    } catch (e) {
      setTs((prev) => (prev ? { ...prev, error: String(e) } : prev));
    } finally {
      setBusy(false);
    }
  }

  async function copyUrl() {
    if (!ts?.publicUrl) return;
    await navigator.clipboard.writeText(ts.publicUrl);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  if (!ts) return <TabShell />;
  if (demo === "missing") return <NotInstalled />;
  if (demo === "offline") return <NotConnected error={null} />;

  // Each state below is a different problem for the user to solve, so they get
  // their own screen rather than one panel with everything disabled.
  if (!ts.installed) return <NotInstalled />;
  if (!ts.connected) return <NotConnected error={ts.error} />;

  return (
    <TabShell>
      <p className="px-0.5 text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
        publishes conduit at a public https address through tailscale funnel, so
        a web agent like gemini spark can reach {machineName}.
      </p>

      <Card>
        <Row
          icon={
            <StatusDot tone={ts.sharing ? "active" : "idle"} size={8} className="mt-0.5" />
          }
          title="share on the internet"
          description={
            ts.sharing
              ? `anyone with the address below can drive ${machineName}`
              : `off — conduit is only reachable from this machine`
          }
        >
          {busy ? (
            <Loader2 size={14} className="animate-spin text-[rgb(var(--text-faint))]" />
          ) : (
            <Switch
              checked={ts.sharing}
              onChange={(v) => void toggle(v)}
              label="Share conduit on the internet"
            />
          )}
        </Row>
      </Card>

      {ts.error && (
        <Card className="border-amber-500/30 bg-amber-500/8">
          <div className="flex gap-2.5 px-3.5 py-2.5">
            <TriangleAlert size={14} className="mt-px shrink-0 text-amber-500" />
            <p className="selectable text-[11px] leading-relaxed break-words text-[rgb(var(--text-dim))]">
              {ts.error}
            </p>
          </div>
        </Card>
      )}

      <AnimatePresence initial={false}>
        {ts.sharing && ts.publicUrl && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
            className="flex flex-col gap-4 overflow-hidden"
          >
            <div className="flex flex-col gap-2">
              <SectionLabel>public endpoint</SectionLabel>
              <Card>
                <div className="flex flex-col gap-2 px-3.5 py-3">
                  <code className="selectable rounded-lg border hairline bg-[rgb(var(--surface-sunken))] px-2.5 py-2 font-mono text-[10.5px] leading-relaxed break-all text-[rgb(var(--text-dim))]">
                    {ts.publicUrl}
                  </code>
                  <div className="flex items-center gap-1.5">
                    <Button variant="primary" onClick={() => void copyUrl()}>
                      {copied ? <Check size={12} /> : <Copy size={12} />}
                      {copied ? "copied" : "copy for gemini spark"}
                    </Button>
                    <Button onClick={() => void rotate()} disabled={busy} title="Issue a new key">
                      <RefreshCw size={12} /> new key
                    </Button>
                  </div>
                </div>
              </Card>
            </div>

            {/* The key is the only thing standing between the internet and this
                machine, so say so plainly rather than burying it. */}
            <Card className="border-[rgb(var(--accent))]/25 bg-[rgb(var(--accent))]/6">
              <div className="flex gap-2.5 px-3.5 py-2.5">
                <ShieldAlert size={14} className="mt-px shrink-0 text-[rgb(var(--accent))]" />
                <p className="text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
                  the random part of that address <em>is</em> the key — anyone who has
                  the full url can use every tool you left enabled. paste it only into
                  gemini spark, and hit <span className="font-medium">new key</span> if
                  it ever leaks.
                </p>
              </div>
            </Card>
          </motion.div>
        )}
      </AnimatePresence>

      <div className="flex flex-col gap-2">
        <SectionLabel>this machine</SectionLabel>
        <Card tone="sunken">
          <Row
            compact
            title="tailscale"
            description={ts.hostname ?? "connected"}
          >
            <span className="flex items-center gap-1 text-[11px] font-medium text-[var(--color-aqua)]">
              <Check size={12} strokeWidth={3} /> connected
            </span>
          </Row>
        </Card>
      </div>
    </TabShell>
  );
}

function NotInstalled() {
  return (
    <TabShell>
      <Card tone="sunken">
        <div className="flex flex-col items-center gap-3 px-8 py-9 text-center">
          <Globe size={22} className="text-[rgb(var(--text-faint))]" />
          <div>
            <p className="text-[13px] font-medium">tailscale isn't installed</p>
            <p className="mt-1 text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
              conduit uses tailscale funnel to publish a public https address without
              opening a port on your router.
            </p>
          </div>
          <a href={DOWNLOAD} target="_blank" rel="noreferrer">
            <Button variant="primary">
              <ExternalLink size={12} /> get tailscale
            </Button>
          </a>
        </div>
      </Card>
    </TabShell>
  );
}

function NotConnected({ error }: { error: string | null }) {
  return (
    <TabShell>
      <Card tone="sunken">
        <div className="flex flex-col items-center gap-3 px-8 py-9 text-center">
          <StatusDot tone="idle" size={9} />
          <div>
            <p className="text-[13px] font-medium">tailscale isn't logged in</p>
            <p className="mt-1 text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
              {error ?? "open the tailscale app and sign in, then come back."}
            </p>
          </div>
        </div>
      </Card>
    </TabShell>
  );
}
