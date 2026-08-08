import { cn } from "@/lib/cn";
import hermesLogo from "@/assets/agents/hermes.png";
import openclawLogo from "@/assets/agents/openclaw.svg";
import opencodeLogo from "@/assets/agents/opencode.svg";

/**
 * The agent's real icon, resolved in three steps.
 *
 * 1. `icon` — a data URL the Rust side reads from the vendor's app installed on
 *    this Mac. Preferred: always current, and nothing is redistributed.
 * 2. A bundled mark, for the CLI-only agents that ship no desktop app. Taken
 *    from each project's own published icon rather than redrawn by hand.
 * 3. A tinted monogram, if a new agent is added without either.
 */
const BUNDLED: Record<string, string> = {
  hermes: hermesLogo,
  openclaw: openclawLogo,
  opencode: opencodeLogo,
};
const TINT: Record<string, [string, string]> = {
  "claude-code": ["#1B6BFF", "#4E9BFF"],
  "claude-desktop": ["#2E7BFF", "#6EC8FF"],
  codex: ["#0A2A6B", "#2E7BFF"],
  hermes: ["#1B6BFF", "#35E6D5"],
  openclaw: ["#2BC9D8", "#6FF3E2"],
  opencode: ["#3D8FFF", "#8FD8FF"],
  gemini: ["#6EC8FF", "#35E6D5"],
};

const FALLBACK: [string, string] = ["#1B6BFF", "#6EC8FF"];

/** First letter of each of the first two words: "claude code" -> "cc". */
function monogram(name: string) {
  return name
    .split(/\s+/)
    .slice(0, 2)
    .map((w) => w[0] ?? "")
    .join("")
    .toLowerCase();
}

export function AgentMark({
  id,
  name,
  icon,
  dimmed = false,
}: {
  id: string;
  name: string;
  icon?: string | null;
  dimmed?: boolean;
}) {
  const src = icon ?? BUNDLED[id];
  if (src) {
    return (
      <img
        src={src}
        alt=""
        aria-hidden
        draggable={false}
        // Rounded because some marks are opaque squares (hermes) while the
        // ones macOS hands us are already squircles — this makes both sit
        // evenly in the row. Transparent marks are unaffected.
        className={cn(
          "size-7 shrink-0 rounded-[7px] object-contain",
          dimmed && "opacity-40 saturate-50",
        )}
      />
    );
  }

  const [from, to] = TINT[id] ?? FALLBACK;
  return (
    <span
      aria-hidden
      className={cn(
        "grid size-7 shrink-0 place-items-center rounded-[8px] text-[10.5px] font-semibold text-white",
        "shadow-[inset_0_1px_0_rgb(255_255_255/0.22)]",
        dimmed && "opacity-40 saturate-50",
      )}
      style={{ backgroundImage: `linear-gradient(140deg, ${from}, ${to})` }}
    >
      {monogram(name)}
    </span>
  );
}
