import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Check, Download, Loader2, Trash2, TriangleAlert } from "lucide-react";
import { AgentMark } from "@/components/AgentMark";
import { Button, Card, Row, SectionLabel, TabShell } from "@/components/Panel";
import { installAgent, listAgents, subscribe, uninstallAgent } from "@/lib/ipc";
import type { AgentTarget, ServerState } from "@/lib/types";
import { cn } from "@/lib/cn";

export function AgentsTab({ server }: { server: ServerState }) {
  const [agents, setAgents] = useState<AgentTarget[]>([]);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    void listAgents().then(setAgents).catch(() => {});
    return subscribe("agents:changed", setAgents);
  }, []);

  async function toggle(a: AgentTarget) {
    setBusy(a.id);
    try {
      const next = a.installed ? await uninstallAgent(a.id) : await installAgent(a.id);
      setAgents((prev) => prev.map((x) => (x.id === next.id ? next : x)));
    } catch {
      void listAgents().then(setAgents).catch(() => {});
    } finally {
      setBusy(null);
    }
  }

  const detected = agents.filter((a) => a.detected);
  const missing = agents.filter((a) => !a.detected);

  return (
    <TabShell>
      <p className="px-0.5 text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
        one click adds conduit to an agent's config. your file is backed up first, and only the
        conduit entry is touched.
      </p>

      <Card>
        {detected.map((a) => (
          <AgentRow
            key={a.id}
            agent={a}
            busy={busy === a.id}
            onToggle={() => void toggle(a)}
            port={server.port}
          />
        ))}
        {detected.length === 0 && (
          <div className="grid place-items-center px-6 py-8 text-center">
            <p className="text-[11px] text-[rgb(var(--text-faint))]">no agents detected yet</p>
          </div>
        )}
      </Card>

      {missing.length > 0 && (
        <div className="flex flex-col gap-2">
          <SectionLabel>not installed</SectionLabel>
          <Card tone="sunken">
            {missing.map((a) => (
              <Row
                key={a.id}
                compact
                icon={<AgentMark id={a.id} name={a.name} icon={a.icon} dimmed />}
                title={<span className="text-[rgb(var(--text-faint))]">{a.name}</span>}
                description={a.error ?? "not installed here"}
              />
            ))}
          </Card>
        </div>
      )}
    </TabShell>
  );
}

function AgentRow({
  agent,
  busy,
  onToggle,
  port,
}: {
  agent: AgentTarget;
  busy: boolean;
  onToggle: () => void;
  port: number;
}) {
  const home = agent.configPath.replace(/^\/Users\/[^/]+/, "~");

  return (
    <Row
      icon={<AgentMark id={agent.id} name={agent.name} icon={agent.icon} />}
      title={agent.name}
      description={
        agent.error ? (
          <span className="flex items-center gap-1 text-amber-600 dark:text-amber-400">
            <TriangleAlert size={10} /> {agent.error}
          </span>
        ) : agent.installed ? (
          <span className="font-mono text-[10.5px]">127.0.0.1:{port}/mcp</span>
        ) : (
          // Config paths get long (Claude Desktop's is 60+ chars). Truncating
          // keeps every row one line tall; the full path is in the tooltip.
          <span className="block truncate font-mono text-[10.5px] opacity-70" title={agent.configPath}>
            {home}
          </span>
        )
      }
    >
      <div className="group relative">
        <AnimatePresence mode="wait" initial={false}>
          {busy ? (
            <motion.span
              key="busy"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="flex h-[26px] items-center px-2.5"
            >
              <Loader2 size={13} className="animate-spin text-[rgb(var(--text-faint))]" />
            </motion.span>
          ) : agent.installed ? (
            <motion.span
              key="installed"
              initial={{ opacity: 0, scale: 0.9 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.9 }}
              transition={{ duration: 0.16 }}
              className="flex items-center"
            >
              {/* Reads as a confirmation until hovered, then offers removal. */}
              <span
                className={cn(
                  "flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-xs font-medium",
                  "text-[var(--color-aqua)] transition-opacity duration-150",
                  "group-hover:pointer-events-none group-hover:opacity-0",
                )}
              >
                <Check size={12} strokeWidth={3} /> installed
              </span>
              <Button
                variant="ghost"
                onClick={onToggle}
                className="absolute inset-0 justify-center opacity-0 transition-opacity duration-150 group-hover:opacity-100"
                title="Remove conduit from this agent"
              >
                <Trash2 size={12} /> remove
              </Button>
            </motion.span>
          ) : (
            <motion.span
              key="install"
              initial={{ opacity: 0, scale: 0.9 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.9 }}
              transition={{ duration: 0.16 }}
            >
              <Button variant="primary" onClick={onToggle}>
                <Download size={12} /> install
              </Button>
            </motion.span>
          )}
        </AnimatePresence>
      </div>
    </Row>
  );
}
