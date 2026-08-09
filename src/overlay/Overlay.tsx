import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "motion/react";
import { getControlState, subscribe } from "@/lib/ipc";
import { isStandalone } from "@/lib/standalone";
import { AiCursor } from "./AiCursor";

/**
 * `?demo=1` outside the Tauri shell forces the overlay on and paints a stand-in
 * desktop behind it, so the glow and cursor can be judged against something
 * other than a transparent page.
 */
const demo =
  isStandalone && new URLSearchParams(window.location.search).get("demo") === "1";

/**
 * Full-screen, click-through decoration shown while an agent is driving.
 *
 * Two layers:
 *  - an aqua vignette breathing around the screen edges, so the takeover is
 *    unmistakable from across the room;
 *  - the AI's cursor, which is the only pointer visible during control.
 *
 * Nothing here is interactive. The window is click-through at both the Tauri
 * and native window level, and `body.conduit-passthrough` kills pointer events in
 * CSS as a third line of defence.
 */
export function Overlay() {
  const [active, setActive] = useState(demo);
  const mounted = useRef(false);

  useEffect(() => {
    if (demo) return;
    mounted.current = true;
    // Pull the current state as well as subscribing: this webview is created
    // hidden at startup and may come up after a session has already begun.
    void getControlState()
      .then((s) => setActive(s.phase === "active"))
      .catch(() => {});
    const off = subscribe("control:state", (s) => setActive(s.phase === "active"));
    return () => {
      mounted.current = false;
      off();
    };
  }, []);

  return (
    <div className="pointer-events-none fixed inset-0 overflow-hidden">
      {demo && <DemoDesktop />}

      <AnimatePresence initial={!demo}>
        {active && (
          <motion.div
            key="glow"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
            className="absolute inset-0"
          >
            <EdgeGlow />
          </motion.div>
        )}
      </AnimatePresence>

      <AiCursor active={active} demo={demo} />
    </div>
  );
}

/** Stand-in desktop for demo screenshots. Never rendered in the real app. */
function DemoDesktop() {
  return (
    <div className="absolute inset-0 -z-10 bg-[linear-gradient(150deg,#1c2b4a,#2a3f63_45%,#3d5a7d)]">
      <div className="absolute top-16 left-24 h-64 w-[420px] rounded-xl border border-white/10 bg-white/8 backdrop-blur-sm" />
      <div className="absolute right-28 bottom-24 h-52 w-[360px] rounded-xl border border-white/10 bg-black/25" />
    </div>
  );
}

/**
 * The border glow. Built from four edge gradients rather than one inset
 * box-shadow: a shadow that large forces the compositor to rasterize the whole
 * screen area every frame, whereas four fixed-size gradient strips stay cheap
 * and let each edge breathe on its own phase.
 */
function EdgeGlow() {
  const common = "absolute animate-breathe";
  return (
    <>
      <div
        className={`${common} inset-x-0 top-0 h-[120px]`}
        style={{
          background:
            "linear-gradient(to bottom, rgb(53 230 213 / 0.55), rgb(27 107 255 / 0.16) 45%, transparent)",
          animationDelay: "0ms",
        }}
      />
      <div
        className={`${common} inset-x-0 bottom-0 h-[120px]`}
        style={{
          background:
            "linear-gradient(to top, rgb(53 230 213 / 0.55), rgb(27 107 255 / 0.16) 45%, transparent)",
          animationDelay: "-1600ms",
        }}
      />
      <div
        className={`${common} inset-y-0 left-0 w-[120px]`}
        style={{
          background:
            "linear-gradient(to right, rgb(53 230 213 / 0.5), rgb(27 107 255 / 0.14) 45%, transparent)",
          animationDelay: "-800ms",
        }}
      />
      <div
        className={`${common} inset-y-0 right-0 w-[120px]`}
        style={{
          background:
            "linear-gradient(to left, rgb(53 230 213 / 0.5), rgb(27 107 255 / 0.14) 45%, transparent)",
          animationDelay: "-2400ms",
        }}
      />
      {/* A crisp hairline pins the effect to the screen edge so it reads as a
          deliberate frame rather than a blurry haze. */}
      <div className="absolute inset-0 border-2 border-[rgb(53_230_213/0.32)]" />
    </>
  );
}
