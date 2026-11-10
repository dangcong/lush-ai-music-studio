/** Original Lush mark: four rising bars and a warm dot. */
export function Logo({ size = 32 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" role="img" aria-label="Lush AI Music Studio">
      <rect width="32" height="32" rx="8" fill="var(--surface-3)" />
      <g fill="var(--accent)">
        <rect x="6" y="13" width="3" height="6" rx="1.5" />
        <rect x="11" y="9" width="3" height="14" rx="1.5" />
        <rect x="16" y="5" width="3" height="22" rx="1.5" />
        <rect x="21" y="10" width="3" height="12" rx="1.5" />
      </g>
      <circle cx="26.5" cy="16" r="1.5" fill="var(--warm)" />
    </svg>
  )
}
