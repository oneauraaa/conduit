import { cn } from "@/lib/cn";

/**
 * The conduit mark: sky over waves, same construction as the app icon but
 * simplified for small sizes. `animated` gently drifts the crests — used in the
 * pill while an agent is driving.
 */
export function Logo({
  size = 22,
  animated = false,
  className,
}: {
  size?: number;
  animated?: boolean;
  className?: string;
}) {
  const uid = animated ? "logo-a" : "logo-s";

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      className={cn("shrink-0", className)}
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={`${uid}-sky`} x1="0.5" y1="0" x2="0.4" y2="1">
          <stop offset="0%" stopColor="#0A2A6B" />
          <stop offset="55%" stopColor="#1B6BFF" />
          <stop offset="100%" stopColor="#6EC8FF" />
        </linearGradient>
        <linearGradient id={`${uid}-w1`} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor="#0A2A6B" />
          <stop offset="100%" stopColor="#2E7BFF" />
        </linearGradient>
        <linearGradient id={`${uid}-w2`} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor="#3D8FFF" />
          <stop offset="100%" stopColor="#8FD8FF" />
        </linearGradient>
        <linearGradient id={`${uid}-w3`} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0%" stopColor="#2BC9D8" />
          <stop offset="100%" stopColor="#6FF3E2" />
        </linearGradient>
        <clipPath id={`${uid}-clip`}>
          <rect width="48" height="48" rx="11.5" />
        </clipPath>
      </defs>

      <g clipPath={`url(#${uid}-clip)`}>
        <rect width="48" height="48" fill={`url(#${uid}-sky)`} />
        <g className={animated ? "logo-waves" : undefined}>
          <path
            d="M-4,30 C4,26 11,32 19,30 C27,28 34,25 42,27 C46,28 50,30 52,29 L52,52 L-4,52 Z"
            fill={`url(#${uid}-w1)`}
          />
          <path
            d="M-4,35 C4,31 11,37 19,35 C27,33 34,30 42,32 C46,33 50,35 52,34 L52,52 L-4,52 Z"
            fill={`url(#${uid}-w2)`}
          />
          <path
            d="M-4,40 C4,36 11,42 19,40 C27,38 34,35 42,37 C46,38 50,40 52,39 L52,52 L-4,52 Z"
            fill={`url(#${uid}-w3)`}
          />
        </g>
      </g>
      <rect
        x="0.5"
        y="0.5"
        width="47"
        height="47"
        rx="11"
        fill="none"
        stroke="#04122E"
        strokeOpacity="0.22"
      />
    </svg>
  );
}
