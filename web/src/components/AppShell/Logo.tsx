// Logo — split from AppShell.tsx; barrel re-exports keep every public path & name


export function Logo({ size = 24 }: { size?: number }) {
  return (
    <svg className="rail-logo" width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
      <rect x="1.5" y="1.5" width="29" height="29" rx="8" fill="none" stroke="var(--accent-line)" />
      <path d="M8 22c4-10 12-10 16 0" stroke="var(--accent)" strokeWidth="2.2" fill="none" strokeLinecap="round" />
      <path d="M16 8v9m0 0-3.2-3.2M16 17l3.2-3.2" stroke="var(--text-1)" strokeWidth="1.8" fill="none" strokeLinecap="round" strokeLinejoin="round" />
      <circle cx="16" cy="23.5" r="1.8" fill="var(--accent)" />
    </svg>
  );
}
