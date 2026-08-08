import { cn } from "@/lib/cn";

export function Switch({
  checked,
  onChange,
  disabled = false,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-[22px] w-[38px] shrink-0 rounded-full",
        "transition-colors duration-200 ease-[var(--ease-out-soft)]",
        "disabled:cursor-not-allowed disabled:opacity-40",
        checked
          ? "bg-[linear-gradient(100deg,var(--color-blue),var(--color-aqua))]"
          : "bg-[rgb(var(--text-faint)/0.32)]",
      )}
    >
      <span
        className={cn(
          "absolute top-[3px] left-[3px] size-4 rounded-full bg-white",
          "shadow-[0_1px_3px_rgb(4_18_46/0.4)]",
          "transition-transform duration-200 ease-[var(--ease-out-soft)]",
          checked && "translate-x-4",
        )}
      />
    </button>
  );
}
