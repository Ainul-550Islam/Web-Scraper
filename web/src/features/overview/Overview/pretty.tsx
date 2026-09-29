// pretty — split from Overview.tsx; barrel re-exports keep every public path & name



export function pretty(s: string): string {
  return s.replace(/_/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}
