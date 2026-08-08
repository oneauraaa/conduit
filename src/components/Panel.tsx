import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

/** Scroll container every tab sits in. Owns the top inset that clears the drag strip. */
export function TabShell({ children }: { children?: ReactNode }) {
  return <div className="flex flex-col gap-4 px-5 pt-9 pb-5">{children}</div>;
}

/**
 * Section heading. Lowercase with wide tracking rather than the usual
 * small-caps treatment — the whole product is lowercase, and uppercase labels
 * fought that everywhere they appeared.
 */
export function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <h2 className="px-0.5 text-[10.5px] font-semibold tracking-[0.06em] text-[rgb(var(--text-faint))] lowercase">
      {children}
    </h2>
  );
}

export function Card({
  children,
  className,
  tone = "default",
}: {
  children: ReactNode;
  className?: string;
  tone?: "default" | "sunken";
}) {
  return (
    <div
      className={cn(
        "overflow-hidden rounded-xl border hairline",
        tone === "sunken" ? "bg-[rgb(var(--surface-sunken)/0.55)]" : "bg-[rgb(var(--surface-raised))]",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** A single settings line: label + optional description on the left, control on the right. */
export function Row({
  title,
  description,
  children,
  icon,
  className,
  compact = false,
}: {
  title: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  icon?: ReactNode;
  className?: string;
  /** Tighter vertical rhythm, for long lists like the tool catalog. */
  compact?: boolean;
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-3 px-3.5",
        compact ? "py-[7px]" : "py-2.5",
        "border-b hairline last:border-b-0",
        className,
      )}
    >
      {icon && <div className="shrink-0 text-[rgb(var(--text-dim))]">{icon}</div>}
      <div className="min-w-0 flex-1">
        <div className="text-[13px] leading-tight font-medium">{title}</div>
        {description && (
          <div className="mt-0.5 text-[11px] leading-snug text-[rgb(var(--text-dim))]">
            {description}
          </div>
        )}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

export function Button({
  children,
  onClick,
  variant = "default",
  disabled = false,
  className,
  title,
}: {
  children: ReactNode;
  onClick?: () => void;
  variant?: "default" | "primary" | "ghost" | "danger";
  disabled?: boolean;
  className?: string;
  title?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium whitespace-nowrap",
        "transition-all duration-150 ease-[var(--ease-out-soft)]",
        "active:scale-[0.97] disabled:pointer-events-none disabled:opacity-40",
        variant === "primary" &&
          "conduit-gradient text-white shadow-[0_1px_5px_rgb(27_107_255/0.4)] hover:brightness-110",
        variant === "default" &&
          "border hairline bg-[rgb(var(--surface))] hover:bg-[rgb(var(--surface-sunken))]",
        variant === "ghost" &&
          "text-[rgb(var(--text-dim))] hover:bg-[rgb(var(--text-faint)/0.14)] hover:text-[rgb(var(--text))]",
        variant === "danger" &&
          "border border-red-500/30 bg-red-500/10 text-red-600 hover:bg-red-500/20 dark:text-red-400",
        className,
      )}
    >
      {children}
    </button>
  );
}
