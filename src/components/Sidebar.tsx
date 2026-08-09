import { motion } from "motion/react";
import { Bot, Server, SlidersHorizontal, Wrench, type LucideIcon } from "lucide-react";
import { TailscaleIcon } from "./TailscaleIcon";
import { cn } from "@/lib/cn";
import { Logo } from "./Logo";
import { ThemeToggle } from "./ThemeToggle";
import { StatusDot } from "./StatusDot";
import type { ServerStatus } from "@/lib/types";

export type Tab = "server" | "tools" | "agents" | "tailscale" | "settings";

/** Tailscale's mark isn't a Lucide icon, so the type allows either. */
type TabIcon = LucideIcon | ((p: { size?: number; className?: string }) => React.ReactElement);

const TABS: { id: Tab; label: string; icon: TabIcon }[] = [
  { id: "server", label: "server", icon: Server },
  { id: "tools", label: "tools", icon: Wrench },
  { id: "agents", label: "agents", icon: Bot },
  { id: "tailscale", label: "tailscale", icon: TailscaleIcon },
  { id: "settings", label: "settings", icon: SlidersHorizontal },
];

export function Sidebar({
  tab,
  onTab,
  serverStatus,
}: {
  tab: Tab;
  onTab: (t: Tab) => void;
  serverStatus: ServerStatus;
}) {
  return (
    <nav className="flex w-[168px] shrink-0 flex-col border-r hairline bg-[rgb(var(--surface-sunken)/0.6)]">
      {/* Brand. Sits under the drag strip, so it moves the window too. */}
      <div data-tauri-drag-region className="flex items-center gap-2 px-4 pt-[13px] pb-4">
        <Logo size={20} />
        <span className="text-[15px] font-semibold tracking-tight lowercase">conduit</span>
      </div>

      <div className="flex flex-1 flex-col gap-0.5 px-2">
        {TABS.map(({ id, label, icon: Icon }) => {
          const active = id === tab;
          return (
            <button
              key={id}
              type="button"
              onClick={() => onTab(id)}
              className={cn(
                "relative flex items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-[13px]",
                "transition-colors duration-150",
                active
                  ? "text-[rgb(var(--text))]"
                  : "text-[rgb(var(--text-dim))] hover:text-[rgb(var(--text))]",
              )}
            >
              {active && (
                <motion.span
                  layoutId="tab-pill"
                  transition={{ type: "spring", stiffness: 480, damping: 40 }}
                  className="absolute inset-0 rounded-lg bg-[rgb(var(--surface))] shadow-[0_1px_2px_rgb(4_18_46/0.08)] hairline border"
                />
              )}
              <Icon
                size={15}
                className={cn(
                  "relative z-10 transition-colors duration-150",
                  active && "text-[rgb(var(--accent))]",
                )}
              />
              <span className="relative z-10 lowercase">{label}</span>

              {/* Server health stays visible from every tab. */}
              {id === "server" && (
                <StatusDot
                  tone={
                    serverStatus === "running"
                      ? "live"
                      : serverStatus === "error"
                        ? "error"
                        : "idle"
                  }
                  size={6}
                  className="relative z-10 ml-auto"
                />
              )}
            </button>
          );
        })}
      </div>

      <div className="flex items-center justify-between px-3 py-2.5">
        <span className="text-[10px] tracking-wide text-[rgb(var(--text-faint))] tabular-nums">
          v0.1.0
        </span>
        <ThemeToggle />
      </div>
    </nav>
  );
}
