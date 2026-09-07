import { useEffect, useRef } from "react";
import { subscribe } from "@/lib/ipc";

/** How long a trail sample stays visible, in ms. */
const TRAIL_MS = 260;
/** Ripples live this long after a click. */
const RIPPLE_MS = 460;

interface Sample {
  x: number;
  y: number;
  t: number;
}
interface Ripple {
  x: number;
  y: number;
  t: number;
  kind: "click" | "key";
}

/**
 * The AI's cursor, drawn on a canvas.
 *
 * Canvas rather than DOM because the trail is ~20 overlapping translucent
 * segments that change every frame; as elements they'd thrash style recalc
 * across the whole screen. Here it's one composited layer.
 *
 * Coordinates arrive already local to this overlay's display — the Rust side
 * subtracts the display origin before emitting, so multi-monitor needs no
 * conversion here.
 */
export function AiCursor({ active, demo = false }: { active: boolean; demo?: boolean }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const pos = useRef<{ x: number; y: number } | null>(null);
  const trail = useRef<Sample[]>([]);
  const ripples = useRef<Ripple[]>([]);
  const raf = useRef(0);
  /** Wakes the draw loop. Set once the canvas effect has run. */
  const wake = useRef<(() => void) | null>(null);

  // In demo mode, fake a swipe into place so the trail and a click ripple are
  // both on screen for the screenshot.
  useEffect(() => {
    if (!demo) return;
    const now = performance.now();
    const cx = window.innerWidth * 0.52;
    const cy = window.innerHeight * 0.46;
    for (let i = 0; i < 14; i++) {
      const t = i / 13;
      trail.current.push({
        x: cx - (1 - t) * 190 + Math.sin(t * 3) * 22,
        y: cy - (1 - t) * 120,
        t: now - (1 - t) * TRAIL_MS * 0.92,
      });
    }
    pos.current = { x: cx, y: cy };
    ripples.current.push({ x: cx, y: cy, t: now - 150, kind: "click" });
  }, [demo]);

  useEffect(() => {
    const offCursor = subscribe("control:cursor", ({ x, y }) => {
      pos.current = { x, y };
      trail.current.push({ x, y, t: performance.now() });
      wake.current?.();
    });
    const offPulse = subscribe("control:pulse", ({ kind }) => {
      const p = pos.current;
      if (p) ripples.current.push({ x: p.x, y: p.y, t: performance.now(), kind });
      wake.current?.();
    });
    return () => {
      offCursor();
      offPulse();
    };
  }, []);

  useEffect(() => {
    if (!active) {
      pos.current = null;
      trail.current = [];
      ripples.current = [];
    }
    // Either way one frame is owed: the one that wipes the last cursor off, or
    // the first of a new session.
    wake.current?.();
  }, [active]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const resize = () => {
      canvas.width = Math.floor(window.innerWidth * dpr);
      canvas.height = Math.floor(window.innerHeight * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    resize();
    window.addEventListener("resize", resize);

    // Whether a frame is already queued. Without it, an event burst — and
    // `control:cursor` arrives once per interpolated step of a glide — would
    // queue one loop per event.
    let queued = false;

    /**
     * Draw one frame, and schedule the next only if something is still moving.
     *
     * The loop used to be unconditional: `requestAnimationFrame` re-armed at
     * the top of `draw`, from mount, forever. That meant a full-screen
     * `clearRect` plus a canvas re-upload every frame for the entire life of
     * the process — while the overlay was hidden, and while a session sat
     * still with the cursor parked. On a 2560×1440 display at 280Hz that is
     * most of a CPU core spent compositing nothing, and it is felt: the glow
     * this canvas sits on top of stutters.
     *
     * The trail and the ripples are the only things that age, so they are
     * exactly the condition for wanting another frame. Everything else that
     * changes the picture — a cursor move, a click, the session ending —
     * arrives as an event, and those call `wake`.
     */
    const draw = () => {
      queued = false;
      const now = performance.now();
      const w = window.innerWidth;
      const h = window.innerHeight;
      ctx.clearRect(0, 0, w, h);

      trail.current = trail.current.filter((s) => now - s.t < TRAIL_MS);
      ripples.current = ripples.current.filter((r) => now - r.t < RIPPLE_MS);

      // ── comet trail ──────────────────────────────────────
      const pts = trail.current;
      if (pts.length > 1) {
        ctx.lineCap = "round";
        ctx.lineJoin = "round";
        for (let i = 1; i < pts.length; i++) {
          const a = pts[i - 1];
          const b = pts[i];
          const age = (now - b.t) / TRAIL_MS;
          const life = 1 - age;
          ctx.strokeStyle = `rgba(53, 230, 213, ${life * 0.5})`;
          ctx.lineWidth = 2 + life * 7;
          ctx.beginPath();
          ctx.moveTo(a.x, a.y);
          ctx.lineTo(b.x, b.y);
          ctx.stroke();
        }
      }

      // ── click / key pulses ───────────────────────────────
      for (const r of ripples.current) {
        const p = (now - r.t) / RIPPLE_MS;
        const eased = 1 - Math.pow(1 - p, 3);
        const radius = r.kind === "click" ? 6 + eased * 34 : 6 + eased * 18;
        ctx.strokeStyle = `rgba(110, 200, 255, ${(1 - p) * 0.75})`;
        ctx.lineWidth = 2.5 * (1 - p) + 0.5;
        ctx.beginPath();
        ctx.arc(r.x, r.y, radius, 0, Math.PI * 2);
        ctx.stroke();
      }

      // ── the cursor itself ────────────────────────────────
      const p = pos.current;
      if (p) {
        // A recent click briefly swells the head.
        const lastClick = ripples.current.at(-1);
        const punch = lastClick ? Math.max(0, 1 - (now - lastClick.t) / 180) : 0;
        const scale = 1 + punch * 0.35;

        // outer bloom
        const bloom = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, 30 * scale);
        bloom.addColorStop(0, "rgba(27, 107, 255, 0.42)");
        bloom.addColorStop(0.55, "rgba(27, 107, 255, 0.14)");
        bloom.addColorStop(1, "rgba(27, 107, 255, 0)");
        ctx.fillStyle = bloom;
        ctx.beginPath();
        ctx.arc(p.x, p.y, 30 * scale, 0, Math.PI * 2);
        ctx.fill();

        // halo
        const halo = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, 15 * scale);
        halo.addColorStop(0, "rgba(110, 200, 255, 0.85)");
        halo.addColorStop(1, "rgba(110, 200, 255, 0)");
        ctx.fillStyle = halo;
        ctx.beginPath();
        ctx.arc(p.x, p.y, 15 * scale, 0, Math.PI * 2);
        ctx.fill();

        // core
        ctx.fillStyle = "rgba(207, 247, 255, 0.98)";
        ctx.beginPath();
        ctx.arc(p.x, p.y, 4.2 * scale, 0, Math.PI * 2);
        ctx.fill();

        ctx.strokeStyle = "rgba(53, 230, 213, 0.95)";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(p.x, p.y, 7 * scale, 0, Math.PI * 2);
        ctx.stroke();
      }

      // Only the ageing things need another frame. A parked cursor is a still
      // picture, and the canvas already holds it.
      if (trail.current.length > 0 || ripples.current.length > 0) schedule();
    };

    const schedule = () => {
      if (queued) return;
      queued = true;
      raf.current = requestAnimationFrame(draw);
    };

    wake.current = schedule;
    // The first frame: whatever is on screen when this mounts.
    schedule();

    return () => {
      wake.current = null;
      cancelAnimationFrame(raf.current);
      window.removeEventListener("resize", resize);
    };
  }, []);

  return <canvas ref={canvasRef} className="absolute inset-0 size-full" />;
}
