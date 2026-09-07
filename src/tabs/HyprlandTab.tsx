import { useEffect, useMemo, useState } from "react";
import { motion } from "motion/react";
import { Layers, Loader2, RefreshCw, Search, TriangleAlert } from "lucide-react";
import { HyprlandIcon } from "@/components/HyprlandIcon";
import { Button, Card, SectionLabel, TabShell } from "@/components/Panel";
import { cn } from "@/lib/cn";
import { getHyprlandState } from "@/lib/ipc";
import type { HyprlandState, Keybind } from "@/lib/types";

/** A single key drawn as a key. */
function Cap({ children, tone = "key" }: { children: string; tone?: "mod" | "key" }) {
  return (
    <kbd
      className={cn(
        "inline-flex min-w-[20px] items-center justify-center rounded-[5px] px-1.5 py-0.5",
        "border hairline font-mono text-[10.5px] leading-none whitespace-nowrap",
        tone === "mod"
          ? "bg-[rgb(var(--surface-sunken))] text-[rgb(var(--text-dim))]"
          : "bg-[rgb(var(--surface))] text-[rgb(var(--text))] shadow-[0_1px_0_rgb(var(--border)/0.18)]",
      )}
    >
      {children}
    </kbd>
  );
}

/** `SUPER + SHIFT + P` as caps, with the `+` kept quiet between them. */
function Chord({ bind }: { bind: Keybind }) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      {bind.mods.map((m) => (
        <span key={m} className="flex items-center gap-1">
          <Cap tone="mod">{m.toLowerCase()}</Cap>
          <span className="text-[10px] text-[rgb(var(--text-faint))]">+</span>
        </span>
      ))}
      <Cap>{bind.key}</Cap>
    </div>
  );
}

function Tag({
  children,
  tone = "dim",
  title,
}: {
  children: React.ReactNode;
  tone?: "dim" | "warn";
  title?: string;
}) {
  return (
    <span
      title={title}
      className={cn(
        "rounded px-1.5 py-px text-[9.5px] leading-[1.5] font-medium whitespace-nowrap lowercase",
        tone === "warn"
          ? "bg-amber-500/12 text-amber-600 dark:text-amber-400"
          : "bg-[rgb(var(--text-faint)/0.13)] text-[rgb(var(--text-dim))]",
      )}
    >
      {children}
    </span>
  );
}

function BindRow({ bind }: { bind: Keybind }) {
  const action = bind.args ? `${bind.dispatcher} ${bind.args}` : bind.dispatcher;

  return (
    <div className="flex items-start gap-3 border-b hairline px-3.5 py-2 last:border-b-0">
      <div className="w-[168px] shrink-0 pt-px">
        <Chord bind={bind} />
      </div>

      <div className="min-w-0 flex-1">
        <div
          className={cn(
            "selectable font-mono text-[11.5px] leading-snug break-all",
            bind.active ? "text-[rgb(var(--text))]" : "text-[rgb(var(--text-faint))]",
          )}
        >
          {action}
        </div>

        {bind.description && (
          <div className="mt-0.5 text-[11px] leading-snug text-[rgb(var(--text-dim))]">
            {bind.description}
          </div>
        )}

        {(bind.submap || !bind.active || bind.alsoFires.length > 0) && (
          <div className="mt-1 flex flex-wrap items-center gap-1">
            {bind.submap && <Tag>submap: {bind.submap}</Tag>}
            {!bind.active && (
              <Tag tone="warn" title="declared in your config, but hyprland has not loaded it">
                not loaded
              </Tag>
            )}
            {/* Hyprland runs every bind that matches rather than stopping at
                the first, so a repeated chord fires all of them. Nearly always
                an accident, and invisible until something points at it. */}
            {bind.alsoFires.length > 0 && (
              <Tag
                tone="warn"
                title={`this chord also runs: ${bind.alsoFires.join(", ")}`}
              >
                also runs {bind.alsoFires.join(", ")}
              </Tag>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export function HyprlandTab() {
  const [state, setState] = useState<HyprlandState | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void getHyprlandState().then(setState).catch(() => {});
  }, []);

  async function refresh() {
    setBusy(true);
    try {
      setState(await getHyprlandState());
    } finally {
      setBusy(false);
    }
  }

  /** Matches the chord, the command and the description alike — people look for
   *  a bind by any of the three, and which one they remember varies. */
  const groups = useMemo(() => {
    if (!state) return [];
    const q = query.trim().toLowerCase();

    const matches = state.binds.filter((b) => {
      if (!q) return true;
      const haystack = [
        ...b.mods,
        b.key,
        b.dispatcher,
        b.args,
        b.description ?? "",
        b.section ?? "",
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(q);
    });

    // Insertion order, so the list reads down the config the way it was written.
    const out = new Map<string, Keybind[]>();
    for (const b of matches) {
      const key = b.section ?? "other";
      const list = out.get(key) ?? [];
      list.push(b);
      out.set(key, list);
    }
    return [...out.entries()];
  }, [state, query]);

  if (!state) return <TabShell />;
  if (!state.available) return <NotHyprland />;

  const total = state.binds.length;
  const shown = groups.reduce((n, [, list]) => n + list.length, 0);

  return (
    <TabShell>
      <p className="px-0.5 text-[11px] leading-relaxed text-[rgb(var(--text-dim))]">
        the shortcuts you have bound in hyprland{state.version ? ` ${state.version}` : ""}.
        agents read this same list through the{" "}
        <span className="font-mono text-[10.5px]">list_keybinds</span> tool, so they
        use your bindings instead of guessing at one.
      </p>

      {/* conduit prefers hyprland.conf; hyprland loads hyprland.lua first. When
          both exist the list would silently be the wrong one, so say so. */}
      {state.mismatch && (
        <Card className="border-amber-500/30 bg-amber-500/8">
          <div className="flex gap-2.5 px-3.5 py-2.5">
            <TriangleAlert size={14} className="mt-px shrink-0 text-amber-500" />
            <p className="selectable text-[11px] leading-relaxed break-words text-[rgb(var(--text-dim))]">
              {state.mismatch}
            </p>
          </div>
        </Card>
      )}

      {state.error && (
        <Card className="border-amber-500/30 bg-amber-500/8">
          <div className="flex gap-2.5 px-3.5 py-2.5">
            <TriangleAlert size={14} className="mt-px shrink-0 text-amber-500" />
            <p className="selectable text-[11px] leading-relaxed break-words text-[rgb(var(--text-dim))]">
              {state.error}
            </p>
          </div>
        </Card>
      )}

      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Search
            size={13}
            className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-[rgb(var(--text-faint))]"
          />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="search shortcuts, commands, notes"
            aria-label="Search keybinds"
            className={cn(
              "w-full rounded-lg border hairline bg-[rgb(var(--surface))] py-1.5 pr-2.5 pl-7.5",
              "text-[12px] placeholder:text-[rgb(var(--text-faint))]",
              "outline-none focus:border-[rgb(var(--accent)/0.5)]",
            )}
          />
        </div>
        <Button onClick={() => void refresh()} disabled={busy} title="Re-read your config">
          {busy ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <RefreshCw size={12} />
          )}
          reload
        </Button>
      </div>

      <div className="flex items-center justify-between px-0.5">
        <SectionLabel>
          {query ? `${shown} of ${total} shortcuts` : `${total} shortcuts`}
        </SectionLabel>
        {state.configPath && (
          <span
            className="selectable truncate pl-3 text-[10px] text-[rgb(var(--text-faint))]"
            title={state.configPath}
          >
            {state.configPath}
          </span>
        )}
      </div>

      {shown === 0 ? (
        <Card tone="sunken">
          <p className="px-3.5 py-6 text-center text-[11.5px] text-[rgb(var(--text-dim))]">
            {total === 0
              ? "no keybinds found in your hyprland config."
              : `nothing matches "${query}".`}
          </p>
        </Card>
      ) : (
        groups.map(([section, list], i) => (
          <motion.div
            key={section}
            initial={{ opacity: 0, y: 4 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.18, delay: Math.min(i, 6) * 0.02, ease: [0.22, 1, 0.36, 1] }}
            className="flex flex-col gap-1.5"
          >
            <SectionLabel>{section}</SectionLabel>
            <Card>
              {list.map((b) => (
                <BindRow key={b.id} bind={b} />
              ))}
            </Card>
          </motion.div>
        ))
      )}
    </TabShell>
  );
}

/**
 * Reachable even though the sidebar entry is grayed — a deep link, or the
 * `?platform=` override. Says what it needs rather than showing an empty list.
 */
function NotHyprland() {
  return (
    <TabShell>
      <Card tone="sunken">
        <div className="flex flex-col items-center gap-2.5 px-6 py-10 text-center">
          <div className="rounded-full bg-[rgb(var(--text-faint)/0.12)] p-3">
            {/* Full colour: at this size the mark reads as itself, and it is
                the one thing on the card doing the explaining. */}
            <HyprlandIcon size={22} gradient />
          </div>
          <h3 className="text-[13px] font-medium">not a hyprland session</h3>
          <p className="max-w-[300px] text-[11.5px] leading-relaxed text-[rgb(var(--text-dim))]">
            this tab reads the keyboard shortcuts you have bound in hyprland, so
            agents use them instead of inventing a key combination. it turns on
            by itself when conduit is running under hyprland.
          </p>
          <div className="mt-1 flex items-center gap-1.5 text-[10.5px] text-[rgb(var(--text-faint))]">
            <Layers size={11} />
            <span>other desktops keep every other tool</span>
          </div>
        </div>
      </Card>
    </TabShell>
  );
}
