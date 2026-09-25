import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/cn";

export interface PickerOption {
  id: string;
  label: string;
}

/**
 * A compact themed dropdown. The native `<select>` popup ignores the app's
 * theme on every platform webview, so this draws its own list — with the
 * listbox semantics and arrow-key movement a select would have had.
 */
export function Picker({
  options,
  selectedId,
  onSelect,
  label,
  listLabel,
  placeholder = "choose",
  disabled = false,
  className,
}: {
  options: PickerOption[];
  selectedId: string;
  onSelect: (id: string) => void;
  /** Accessible name of the trigger button. */
  label: string;
  /** Accessible name of the open list. */
  listLabel: string;
  placeholder?: string;
  disabled?: boolean;
  /** Sizing for the trigger; defaults to a fixed narrow width. */
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const selected = options.find((option) => option.id === selectedId);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [open]);

  function moveFocus(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      setOpen(false);
      trigger.current?.focus();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const items = Array.from(root.current?.querySelectorAll<HTMLButtonElement>('[role="option"]') ?? []);
      const index = items.indexOf(document.activeElement as HTMLButtonElement);
      items[(index + (event.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
    }
  }

  return (
    <div ref={root} className="relative min-w-0" onKeyDown={moveFocus}>
      <button
        ref={trigger}
        type="button"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((value) => !value)}
        className={cn(
          "flex w-[112px] items-center justify-between gap-2 rounded-lg border hairline bg-[rgb(var(--surface-sunken))] px-2.5 py-1.5 text-[11px] text-[rgb(var(--text))] transition-colors hover:bg-[rgb(var(--surface))] focus-visible:border-[rgb(var(--accent))] focus-visible:outline-none disabled:opacity-40",
          className,
        )}
      >
        <span className="truncate">{selected?.label ?? placeholder}</span>
        <ChevronDown size={12} className={cn("shrink-0 text-[rgb(var(--text-faint))] transition-transform", open && "rotate-180")} />
      </button>
      {open && (
        <div role="listbox" aria-label={listLabel} className="absolute top-[calc(100%+5px)] right-0 z-30 min-w-[160px] overflow-hidden rounded-lg border hairline bg-[rgb(var(--surface-raised))] p-1 shadow-[0_6px_14px_rgb(4_18_46/0.3)]">
          {options.map((option) => (
            <button
              key={option.id}
              type="button"
              role="option"
              aria-selected={option.id === selectedId}
              onClick={() => { setOpen(false); onSelect(option.id); trigger.current?.focus(); }}
              className={cn(
                "flex w-full items-center rounded-md px-2.5 py-1.5 text-left text-[11px] focus-visible:outline-none focus-visible:bg-[rgb(var(--accent)/0.12)]",
                option.id === selectedId
                  ? "bg-[rgb(var(--accent)/0.13)] font-medium text-[rgb(var(--accent))]"
                  : "text-[rgb(var(--text))] hover:bg-[rgb(var(--surface-sunken))]",
              )}
            >
              {option.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
