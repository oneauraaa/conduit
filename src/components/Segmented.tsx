import { motion } from "motion/react";
import { useId } from "react";
import { cn } from "@/lib/cn";

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  hint?: string;
}

/**
 * Segmented control with a gradient thumb that slides between options.
 * The thumb is a shared `layoutId`, so switching tabs animates rather than
 * repaints.
 */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled = false,
  size = "md",
}: {
  value: T;
  options: readonly SegmentOption<T>[];
  onChange: (next: T) => void;
  disabled?: boolean;
  size?: "sm" | "md";
}) {
  const id = useId();

  return (
    <div
      role="radiogroup"
      className={cn(
        "relative inline-flex shrink-0 items-center rounded-lg p-[3px]",
        "bg-[rgb(var(--surface-sunken))] hairline border",
        disabled && "opacity-45",
      )}
    >
      {options.map((opt) => {
        const active = opt.value === value;
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={disabled}
            title={opt.hint}
            onClick={() => onChange(opt.value)}
            className={cn(
              "relative rounded-[6px] font-medium whitespace-nowrap",
              "transition-colors duration-150",
              size === "sm" ? "px-2.5 py-1 text-[11px]" : "px-3 py-1.5 text-xs",
              disabled ? "cursor-not-allowed" : "cursor-default",
              active ? "text-white" : "text-[rgb(var(--text-dim))] hover:text-[rgb(var(--text))]",
            )}
          >
            {active && (
              <motion.span
                layoutId={`seg-${id}`}
                transition={{ type: "spring", stiffness: 520, damping: 38 }}
                className="conduit-gradient absolute inset-0 rounded-[5px] shadow-[0_1px_4px_rgb(27_107_255/0.4)]"
              />
            )}
            <span className="relative z-10">{opt.label}</span>
          </button>
        );
      })}
    </div>
  );
}
