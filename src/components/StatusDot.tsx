import { cn } from "@/lib/cn";

export type DotTone = "idle" | "live" | "active" | "error";

const TONE: Record<DotTone, { core: string; halo: string }> = {
  idle: { core: "bg-[rgb(var(--text-faint))]", halo: "" },
  live: { core: "bg-[var(--color-aqua)]", halo: "bg-[var(--color-aqua)]" },
  active: { core: "bg-amber-400", halo: "bg-amber-400" },
  error: { core: "bg-red-500", halo: "bg-red-500" },
};

/** Small state light. `live` and `active` breathe; `idle` and `error` sit still. */
export function StatusDot({
  tone,
  size = 8,
  className,
}: {
  tone: DotTone;
  size?: number;
  className?: string;
}) {
  const { core, halo } = TONE[tone];
  const breathing = tone === "live" || tone === "active";

  return (
    <span
      className={cn("relative inline-flex shrink-0", className)}
      style={{ width: size, height: size }}
    >
      {breathing && (
        <span
          className={cn("absolute inset-0 rounded-full opacity-40 animate-breathe blur-[3px]", halo)}
          style={{ transform: "scale(2)" }}
        />
      )}
      <span className={cn("relative size-full rounded-full", core)} />
    </span>
  );
}
