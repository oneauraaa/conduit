import { useId } from "react";

/**
 * Hyprland's mark — the two mirrored hooks, path taken verbatim from the
 * project's own favicon so the shape is theirs rather than an approximation.
 *
 * Monochrome by default, for the same reason [`TailscaleIcon`] is: a sidebar
 * icon has to pick up the active/inactive tab colour through `currentColor`, so
 * a fixed brand gradient there would be the one icon that ignores the theme.
 * The gradient is Hyprland's own, kept for `gradient` — worth it at the size
 * the empty state draws it, where the logo is the subject rather than a label.
 *
 * The viewBox is the one thing not taken from the source. Hyprland's favicon is
 * cropped tight to the mark, and a tight box next to Lucide's — whose glyphs sit
 * inside 20 of their 24 units — renders this ~20% taller than every icon beside
 * it. So the box is squared off and padded to put the mark at the same 83%,
 * which is measured against `server` and `tailscale` rather than guessed.
 */
export function HyprlandIcon({
  size = 15,
  className,
  gradient = false,
}: {
  size?: number;
  className?: string;
  gradient?: boolean;
}) {
  // Two of these can be mounted at once — the sidebar's and the empty state's —
  // and a hardcoded gradient id would have the first one win for both.
  const id = useId();

  return (
    <svg
      width={size}
      height={size}
      viewBox="-67.87 297.96 376.98 376.98"
      fill={gradient ? `url(#${id})` : "currentColor"}
      className={className}
      aria-hidden="true"
    >
      {gradient && (
        <defs>
          <linearGradient id={id} x1="10%" y1="90%" x2="90%" y2="10%">
            <stop offset="0%" stopColor="rgb(0, 168, 244)" />
            <stop offset="100%" stopColor="rgb(0, 229, 208)" />
          </linearGradient>
        </defs>
      )}
      <path d="M107 330c-25.9 39.39-61.43 86.28-79.3 114.97C9.83 473.67-1.86 502.32.24 536.26c3.85 62.1 56.46 106.63 120.38 106.63v-27.4c-51.66 0-90.07-33.06-93.03-80.93-1.67-26.96 6.9-48.66 23.37-75.1 14.07-22.6 33.9-47.95 56.03-80.2zm27.25 0c25.89 39.39 61.42 86.28 79.29 114.97 17.87 28.7 29.56 57.35 27.46 91.29-3.85 62.1-56.46 106.63-120.38 106.63v-27.4c51.66 0 90.07-33.06 93.03-80.93 1.67-26.96-6.9-48.66-23.37-75.1-14.07-22.6-33.9-47.95-56.03-80.2z" />
    </svg>
  );
}
