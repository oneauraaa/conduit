import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  Globe,
  PanelBottom,
  Plus,
  Power,
  ShieldAlert,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { Button, Card, Row, SectionLabel, TabShell } from "@/components/Panel";
import { Switch } from "@/components/Switch";
import {
  getSettings,
  setCorsEnabled,
  setCorsOrigins,
  setStartHidden,
  setStartOnLogin,
  subscribe,
} from "@/lib/ipc";
import type { Settings } from "@/lib/types";
import { cn } from "@/lib/cn";

/**
 * An origin is scheme + host + port, and nothing else. A browser sends exactly
 * that, so anything with a path or a trailing wildcard would silently never
 * match — better to refuse it at the point of typing than to leave the user
 * wondering why their allowlist does nothing.
 */
function validate(raw: string, existing: string[]): string | null {
  const value = raw.trim().replace(/\/+$/, "");
  if (!value) return "enter an origin";

  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return "not a valid origin — include the scheme, e.g. http://127.0.0.1:8080";
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return "only http:// and https:// origins can be allowed";
  }
  if (url.pathname !== "/" || url.search || url.hash) {
    return "an origin is just scheme, host and port — drop the path";
  }
  if (value.includes("*")) {
    return "wildcards are not supported; add each origin explicitly";
  }
  if (existing.some((o) => o.toLowerCase() === value.toLowerCase())) {
    return "already on the list";
  }
  return null;
}

export function SettingsTab() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [startError, setStartError] = useState<string | null>(null);

  useEffect(() => {
    void getSettings().then(setSettings).catch(() => {});
    return subscribe("settings:changed", setSettings);
  }, []);

  const origins = useMemo(() => settings?.corsOrigins ?? [], [settings]);
  const enabled = settings?.corsEnabled ?? false;
  const startOnLogin = settings?.startOnLogin ?? false;
  const startHidden = settings?.startHidden ?? false;

  async function toggleStartOnLogin(next: boolean) {
    try {
      setSettings(await setStartOnLogin(next));
      setStartError(null);
    } catch (e) {
      // The OS can refuse to write the login entry. Say so rather than leaving
      // a switch that flipped but changed nothing.
      setStartError(String(e));
    }
  }

  async function toggleStartHidden(next: boolean) {
    setSettings(await setStartHidden(next));
  }

  async function toggle(next: boolean) {
    setSettings(await setCorsEnabled(next));
  }

  async function add() {
    const problem = validate(draft, origins);
    if (problem) {
      setError(problem);
      return;
    }
    const value = draft.trim().replace(/\/+$/, "");
    setSettings(await setCorsOrigins([...origins, value]));
    setDraft("");
    setError(null);
  }

  async function remove(origin: string) {
    setSettings(await setCorsOrigins(origins.filter((o) => o !== origin)));
  }

  return (
    <TabShell>
      <div className="flex flex-col gap-2">
        <SectionLabel>startup</SectionLabel>

        <Card>
          <Row
            icon={
              <span className={startOnLogin ? "text-[var(--color-aqua)]" : "text-[rgb(var(--text-faint))]"}>
                <Power size={15} />
              </span>
            }
            title="start conduit at login"
            description={
              startError ??
              "the server comes up with the app, so the endpoint is there without remembering to start it"
            }
            className={startOnLogin ? "border-b hairline" : undefined}
          >
            <Switch
              checked={startOnLogin}
              onChange={(next) => void toggleStartOnLogin(next)}
              label="start conduit at login"
            />
          </Row>

          {/* Only offered once the first is on: on its own it would describe a
              launch that never happens. */}
          <AnimatePresence initial={false}>
            {startOnLogin && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
                className="overflow-hidden"
              >
                <Row
                  icon={<PanelBottom size={15} className="text-[rgb(var(--text-faint))]" />}
                  title="start in the tray"
                  description="no window at login — opening conduit yourself still shows it"
                >
                  <Switch
                    checked={startHidden}
                    onChange={(next) => void toggleStartHidden(next)}
                    label="start in the tray"
                  />
                </Row>
              </motion.div>
            )}
          </AnimatePresence>
        </Card>
      </div>

      {/* One setting, not two. The allowlist is the *body* of "allow browser
          clients" rather than a section of its own — giving it its own heading
          read as a second, independent option, and left a stranded list of
          addresses on screen when the switch was off. */}
      <div className="flex flex-col gap-2">
        <SectionLabel>browser access</SectionLabel>

        <Card>
          <Row
            icon={
              <span className={enabled ? "text-amber-500" : "text-[rgb(var(--text-faint))]"}>
                <Globe size={15} />
              </span>
            }
            title="allow browser clients"
            description={
              enabled
                ? "only the addresses below can connect"
                : "off — anything running in a browser is refused"
            }
            className={enabled ? "border-b hairline" : undefined}
          >
            <Switch
              checked={enabled}
              onChange={(next) => void toggle(next)}
              label="allow browser clients"
            />
          </Row>

          {/* Everything below is hidden while the switch is off: with browser
              access disabled the addresses do nothing, and showing a list of
              inert URLs invites the reader to think they are still in force. */}
          <AnimatePresence initial={false}>
            {enabled && (
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
                className="overflow-hidden"
              >
                <div className="flex items-center gap-2 px-3.5 py-2.5">
                  <input
                    value={draft}
                    onChange={(e) => {
                      setDraft(e.target.value);
                      setError(null);
                    }}
                    onKeyDown={(e) => e.key === "Enter" && void add()}
                    placeholder="http://127.0.0.1:8080"
                    spellCheck={false}
                    autoCapitalize="off"
                    autoComplete="off"
                    className={cn(
                      "min-w-0 flex-1 rounded-md border hairline bg-[rgb(var(--surface))] px-2 py-1",
                      "font-mono text-[11px] text-[rgb(var(--text))] outline-none",
                      "placeholder:text-[rgb(var(--text-faint))]",
                      error ? "border-amber-500/60" : "focus:border-[rgb(var(--accent))]",
                    )}
                  />
                  <Button onClick={() => void add()}>
                    <Plus size={11} /> add
                  </Button>
                </div>

                {error && (
                  <div className="flex items-center gap-1.5 px-3.5 pb-2 text-[11px] text-amber-500">
                    <TriangleAlert size={11} className="shrink-0" />
                    {error}
                  </div>
                )}

                {origins.length === 0 ? (
                  <p className="px-3.5 pb-3 text-[11px] leading-relaxed text-[rgb(var(--text-faint))]">
                    nothing allowed yet — a browser client stays refused until
                    its address is on this list.
                  </p>
                ) : (
                  <AnimatePresence initial={false}>
                    {origins.map((origin) => (
                      <motion.div
                        key={origin}
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0, height: 0 }}
                        className="overflow-hidden border-t hairline"
                      >
                        <Row
                          compact
                          title={<span className="font-mono text-[11px]">{origin}</span>}
                        >
                          <button
                            type="button"
                            onClick={() => void remove(origin)}
                            aria-label={`remove ${origin}`}
                            className={cn(
                              "grid size-6 place-items-center rounded-md",
                              "text-[rgb(var(--text-faint))] transition-colors duration-150",
                              "hover:bg-[rgb(var(--text-faint)/0.14)] hover:text-[rgb(var(--text))]",
                            )}
                          >
                            <Trash2 size={12} />
                          </button>
                        </Row>
                      </motion.div>
                    ))}
                  </AnimatePresence>
                )}
              </motion.div>
            )}
          </AnimatePresence>
        </Card>

        {/* The warning stays for as long as the switch is on, rather than being
            a one-time confirmation: leaving an origin allowed is an ongoing
            decision, not a moment. */}
        <AnimatePresence initial={false}>
          {enabled && (
            <motion.div
              initial={{ opacity: 0, height: 0 }}
              animate={{ opacity: 1, height: "auto" }}
              exit={{ opacity: 0, height: 0 }}
              transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
              className="overflow-hidden"
            >
              <Card tone="sunken">
                <div className="flex gap-2.5 px-3.5 py-3">
                  <ShieldAlert size={15} className="mt-px shrink-0 text-amber-500" />
                  <p className="text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
                    a page on an allowed address can use every tool you have
                    switched on — including <span className="font-medium">run_shell</span> —
                    with no further prompt. only add addresses you serve
                    yourself. agents outside a browser are unaffected either way.
                  </p>
                </div>
              </Card>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </TabShell>
  );
}
