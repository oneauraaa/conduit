/**
 * Tailscale's mark: a 3×3 grid of dots.
 *
 * Drawn monochrome and uniform so it sits correctly beside the Lucide icons in
 * the sidebar — `currentColor` lets it pick up the active/inactive tab colour
 * like every other tab. The brand version varies the dots' opacity; a nav icon
 * at 15px can't carry that, and a flat grid stays legible.
 */
export function TailscaleIcon({
  size = 15,
  className,
}: {
  size?: number;
  className?: string;
}) {
  const positions = [5, 12, 19];

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      className={className}
      aria-hidden="true"
    >
      {positions.map((y) =>
        positions.map((x) => <circle key={`${x}-${y}`} cx={x} cy={y} r="2.6" />),
      )}
    </svg>
  );
}
