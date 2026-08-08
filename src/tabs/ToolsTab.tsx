import { useEffect, useMemo, useState } from "react";
import { motion } from "motion/react";
import { Accessibility, Eye, Keyboard, Lock, Terminal, AppWindow } from "lucide-react";
import { Card, Row, SectionLabel, TabShell } from "@/components/Panel";
import { Segmented, type SegmentOption } from "@/components/Segmented";
import { Switch } from "@/components/Switch";
import {
  getSettings,
  getToolCatalog,
  setDefaultAccess,
  setToolEnabled,
  setToolsAccess,
  subscribe,
} from "@/lib/ipc";
import type { AccessMode, Settings, ToolDef, ToolGroup, ToolsAccess } from "@/lib/types";

const ACCESS_OPTIONS: readonly SegmentOption<AccessMode>[] = [
  { value: "manual", label: "manual", hint: "Every action waits for your approval" },
  { value: "auto", label: "auto", hint: "Safe actions run; risky ones ask" },
  { value: "full", label: "full access", hint: "Nothing asks" },
];

const TOOLS_ACCESS_OPTIONS: readonly SegmentOption<ToolsAccess>[] = [
  { value: "all", label: "all tools", hint: "Every tool enabled" },
  { value: "custom", label: "custom", hint: "Pick tools individually below" },
  { value: "off", label: "off", hint: "Block every tool" },
];

const GROUPS: { id: ToolGroup; label: string; icon: typeof Eye }[] = [
  { id: "vision", label: "vision", icon: Eye },
  { id: "input", label: "input", icon: Keyboard },
  { id: "windows", label: "windows & apps", icon: AppWindow },
  { id: "accessibility", label: "accessibility", icon: Accessibility },
  { id: "system", label: "system", icon: Terminal },
];

export function ToolsTab() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [catalog, setCatalog] = useState<ToolDef[]>([]);

  useEffect(() => {
    void getSettings().then(setSettings).catch(() => {});
    void getToolCatalog().then(setCatalog).catch(() => {});
    return subscribe("settings:changed", setSettings);
  }, []);

  const byGroup = useMemo(() => {
    const m = new Map<ToolGroup, ToolDef[]>();
    for (const t of catalog) {
      const list = m.get(t.group) ?? [];
      list.push(t);
      m.set(t.group, list);
    }
    return m;
  }, [catalog]);

  if (!settings) return <TabShell />;

  // In `all` and `off` the per-tool switches are decorative — the master gate
  // already decided. Only `custom` hands control back to the individual rows.
  const locked = settings.toolsAccess !== "custom";
  const isOn = (t: ToolDef) =>
    settings.toolsAccess === "all"
      ? true
      : settings.toolsAccess === "off"
        ? false
        : (settings.toolToggles[t.name] ?? true);

  return (
    <TabShell>
      {/* ── the two master rows ────────────────────────────── */}
      <Card>
        <Row
          title="computer use default access"
          description="how much a new session may do before it stops to ask you"
        >
          <Segmented
            value={settings.defaultAccess}
            options={ACCESS_OPTIONS}
            onChange={(m) => void setDefaultAccess(m)}
            size="sm"
          />
        </Row>
        <Row
          title={
            <span className="flex items-center gap-1.5">
              tools access
              <Lock size={10} className="text-[rgb(var(--text-faint))]" />
            </span>
          }
          description="the master gate over every tool — changeable only here, never from the pill"
        >
          <Segmented
            value={settings.toolsAccess}
            options={TOOLS_ACCESS_OPTIONS}
            onChange={(a) => void setToolsAccess(a)}
            size="sm"
          />
        </Row>
      </Card>

      {/* ── the catalog ────────────────────────────────────── */}
      {GROUPS.map(({ id, label, icon: Icon }) => {
        const tools = byGroup.get(id);
        if (!tools?.length) return null;

        return (
          <div key={id} className="flex flex-col gap-2">
            <SectionLabel>
              <span className="flex items-center gap-1.5">
                <Icon size={11} strokeWidth={2.5} />
                {label}
              </span>
            </SectionLabel>
            <motion.div animate={{ opacity: settings.toolsAccess === "off" ? 0.5 : 1 }}>
              <Card>
                {tools.map((t) => (
                  <Row
                    key={t.name}
                    compact
                    title={
                      <span className="flex items-center gap-1.5 font-mono text-[12px]">
                        {t.name}
                        {t.risky && (
                          <span
                            title="Asks for confirmation in auto mode"
                            className="size-1.5 rounded-full bg-amber-400"
                          />
                        )}
                      </span>
                    }
                    description={t.summary}
                  >
                    <Switch
                      checked={isOn(t)}
                      disabled={locked}
                      onChange={(v) => void setToolEnabled(t.name, v)}
                      label={t.name}
                    />
                  </Row>
                ))}
              </Card>
            </motion.div>
          </div>
        );
      })}
    </TabShell>
  );
}
