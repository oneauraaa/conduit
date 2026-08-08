import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { Check, ChevronUp, Lock, Square, X } from "lucide-react";
import { Logo } from "@/components/Logo";
import {
  getControlState,
  resolveApproval,
  setSessionMode,
  stopControl,
  subscribe,
} from "@/lib/ipc";
import type { AccessMode, ControlState, PendingApproval } from "@/lib/types";
import { isStandalone } from "@/lib/standalone";
import { cn } from "@/lib/cn";

/**
 * Outside the Tauri shell, `?demo=` forces the pill into a state so its
 * appearance can be reviewed in a browser: `?demo=active`, `?demo=open`
 * (dropdown showing) or `?demo=approval`.
 */
const demo = isStandalone
  ? new URLSearchParams(window.location.search).get("demo")
  : null;

const DEMO_APPROVAL: PendingApproval = {
  id: "demo",
  tool: "run_shell",
  summary: "wants to run a shell command",
  detail: "rm -rf ./build && pnpm build",
  agent: "claude code",
};

const MODES: { value: AccessMode; label: string; blurb: string }[] = [
  { value: "manual", label: "manual mode", blurb: "ask me before every action" },
  { value: "auto", label: "auto mode", blurb: "do what's safe, ask when it isn't" },
  { value: "full", label: "full access", blurb: "never ask" },
];

const MODE_LABEL: Record<AccessMode, string> = {
  manual: "manual",
  auto: "auto",
  full: "full access",
};

/**
 * Floats just above the Dock while an agent is driving.
 *
 * The window is sized to the pill and repositioned by Rust from
 * NSScreen.visibleFrame, so it tracks the Dock wherever it lives.
 *
 * The mode dropdown can loosen or tighten *this session's* behaviour, but
 * deliberately cannot touch which tools exist — that gate lives in the Tools
 * tab only, so a running agent can never widen its own reach.
 */
export function Pill() {
  const [control, setControl] = useState<ControlState>({
    phase: demo ? "active" : "idle",
    agent: demo ? "claude code" : null,
    mode: "auto",
    action: demo ? "clicking" : null,
  });
  const [approval, setApproval] = useState<PendingApproval | null>(
    demo === "approval" ? DEMO_APPROVAL : null,
  );
  const [open, setOpen] = useState(demo === "open");

  useEffect(() => {
    // A forced demo state must not be overwritten by the initial sync.
    if (demo) return;

    void getControlState().then(setControl).catch(() => {});
    const offState = subscribe("control:state", (s) => {
      setControl(s);
      if (s.phase === "idle") setOpen(false);
    });
    const offApproval = subscribe("control:approval", setApproval);
    return () => {
      offState();
      offApproval();
    };
  }, []);

  const active = control.phase === "active";

  return (
    <div className="flex h-full w-full items-end justify-center overflow-hidden pb-1">
      {/* Skipping the entry animation in demo mode keeps headless screenshots
          from catching the pill mid-flight at opacity 0. */}
      <AnimatePresence initial={!demo}>
        {active && (
          <motion.div
            key="pill"
            initial={{ y: 64, opacity: 0, scale: 0.92 }}
            animate={{ y: 0, opacity: 1, scale: 1 }}
            exit={{ y: 64, opacity: 0, scale: 0.92 }}
            transition={{ type: "spring", stiffness: 420, damping: 34, mass: 0.8 }}
            className="relative flex flex-col items-stretch"
          >
            {/* The glow is a sibling behind the pill so blur never touches text. */}
            <div
              aria-hidden
              className="pointer-events-none absolute -inset-3 rounded-[28px] opacity-80 blur-xl"
              style={{
                background:
                  "linear-gradient(100deg, rgb(27 107 255 / 0.55), rgb(53 230 213 / 0.55))",
              }}
            />

            {/* Both branches must be motion elements directly under
                AnimatePresence — it needs a ref on its child to drive the
                enter/exit handoff, and a plain component swallows it. */}
            <AnimatePresence mode="wait" initial={false}>
              {approval ? (
                <motion.div
                  key="approval"
                  initial={{ opacity: 0, scale: 0.95 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0, scale: 0.95 }}
                  transition={{ type: "spring", stiffness: 420, damping: 34 }}
                  className="relative w-[352px] overflow-hidden rounded-3xl border border-white/14 bg-[rgb(6_14_30/0.94)] shadow-[0_12px_40px_rgb(0_0_0/0.5)] backdrop-blur-2xl"
                >
                  <ApprovalCard approval={approval} />
                </motion.div>
              ) : (
                <motion.div
                  key="bar"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.14 }}
                  className="relative"
                >
                  {/* dropdown, expanding upward */}
                  <AnimatePresence>
                    {open && (
                      <motion.div
                        initial={{ opacity: 0, y: 10, scale: 0.95 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: 10, scale: 0.95 }}
                        transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
                        className="absolute bottom-[calc(100%+8px)] left-1/2 w-[248px] -translate-x-1/2 overflow-hidden rounded-2xl border border-white/12 bg-[rgb(6_14_30/0.94)] shadow-[0_16px_48px_rgb(0_0_0/0.5)] backdrop-blur-2xl"
                      >
                        {MODES.map((m, i) => (
                          <motion.button
                            key={m.value}
                            type="button"
                            initial={{ opacity: 0, x: -8 }}
                            animate={{ opacity: 1, x: 0 }}
                            transition={{ delay: 0.03 * i, duration: 0.18 }}
                            onClick={() => {
                              void setSessionMode(m.value);
                              setOpen(false);
                            }}
                            className={cn(
                              "flex w-full items-center gap-2.5 px-3 py-2.5 text-left transition-colors",
                              "border-b border-white/8 last:border-b-0 hover:bg-white/8",
                            )}
                          >
                            <span className="flex size-4 shrink-0 items-center justify-center">
                              {control.mode === m.value && (
                                <Check size={13} strokeWidth={3} className="text-[var(--color-aqua)]" />
                              )}
                            </span>
                            <span className="min-w-0 flex-1">
                              <span className="block text-[12.5px] font-medium text-white lowercase">
                                {m.label}
                              </span>
                              <span className="block text-[10.5px] text-white/55 lowercase">
                                {m.blurb}
                              </span>
                            </span>
                          </motion.button>
                        ))}
                        {/* The pill can loosen the mode, but never the tool
                            allowlist — that stays in the app on purpose. */}
                        <div className="flex items-center gap-1.5 bg-white/4 px-3 py-2 text-[10px] whitespace-nowrap text-white/40">
                          <Lock size={9} className="shrink-0" />
                          tool access is set in conduit → tools
                        </div>
                      </motion.div>
                    )}
                  </AnimatePresence>

                  <div className="relative flex items-center gap-2.5 rounded-full border border-white/14 bg-[rgb(6_14_30/0.9)] py-1.5 pr-1.5 pl-3 shadow-[0_8px_28px_rgb(0_0_0/0.45)] backdrop-blur-2xl">
                    <span className="relative flex items-center">
                      <span className="absolute inset-0 animate-breathe rounded-full bg-[var(--color-aqua)] opacity-50 blur-md" />
                      <Logo size={17} animated className="relative" />
                    </span>

                    <span className="flex items-baseline gap-1.5 whitespace-nowrap">
                      <span className="text-[12.5px] font-semibold text-white lowercase">
                        {control.agent ?? "an agent"}
                      </span>
                      <AnimatePresence mode="wait" initial={false}>
                        <motion.span
                          key={control.action ?? "idle"}
                          initial={{ opacity: 0, y: 4 }}
                          animate={{ opacity: 1, y: 0 }}
                          exit={{ opacity: 0, y: -4 }}
                          transition={{ duration: 0.16 }}
                          className="text-[11.5px] text-[var(--color-sky)] lowercase"
                        >
                          {control.action ?? "is in control"}
                        </motion.span>
                      </AnimatePresence>
                    </span>

                    <span className="mx-0.5 h-4 w-px bg-white/15" />

                    <button
                      type="button"
                      onClick={() => setOpen((o) => !o)}
                      className="flex items-center gap-1 rounded-full px-2 py-1 text-[11px] font-medium text-white/80 transition-colors hover:bg-white/10 hover:text-white"
                      title="Change how much this session may do"
                    >
                      {MODE_LABEL[control.mode]}
                      <motion.span animate={{ rotate: open ? 180 : 0 }} transition={{ duration: 0.2 }}>
                        <ChevronUp size={12} />
                      </motion.span>
                    </button>

                    <button
                      type="button"
                      onClick={() => void stopControl()}
                      title="Stop — hand control back to me"
                      className="grid size-7 place-items-center rounded-full bg-white/10 text-white/85 transition-all hover:bg-red-500/85 hover:text-white active:scale-95"
                    >
                      <Square size={11} strokeWidth={3} className="fill-current" />
                    </button>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Manual mode (and risky calls in auto) grow the pill into this. */
function ApprovalCard({ approval }: { approval: PendingApproval }) {
  return (
    <>
      <div className="flex items-start gap-2.5 px-3.5 pt-3 pb-2.5">
        <span className="relative mt-0.5 flex items-center">
          <span className="absolute inset-0 animate-breathe rounded-full bg-amber-400 opacity-50 blur-md" />
          <Logo size={17} className="relative" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-[12.5px] leading-tight font-medium text-white lowercase">
            <span className="font-semibold">{approval.agent ?? "an agent"}</span>{" "}
            {approval.summary}
          </p>
          {approval.detail && (
            <p className="mt-0.5 truncate font-mono text-[10.5px] text-white/50">
              {approval.detail}
            </p>
          )}
        </div>
      </div>

      <div className="flex items-center gap-1.5 border-t border-white/10 bg-white/4 px-2.5 py-2">
        <button
          type="button"
          onClick={() => void resolveApproval(approval.id, "allow")}
          className="conduit-gradient flex-1 rounded-lg px-3 py-1.5 text-[11.5px] font-semibold text-white transition-all hover:brightness-110 active:scale-[0.98]"
        >
          allow
        </button>
        <button
          type="button"
          onClick={() => void resolveApproval(approval.id, "session")}
          className="rounded-lg bg-white/10 px-3 py-1.5 text-[11.5px] font-medium text-white/85 transition-colors hover:bg-white/16"
        >
          allow for session
        </button>
        <button
          type="button"
          onClick={() => void resolveApproval(approval.id, "deny")}
          title="Deny"
          className="grid size-[27px] place-items-center rounded-lg bg-white/10 text-white/70 transition-colors hover:bg-red-500/80 hover:text-white"
        >
          <X size={13} strokeWidth={2.5} />
        </button>
      </div>
    </>
  );
}
