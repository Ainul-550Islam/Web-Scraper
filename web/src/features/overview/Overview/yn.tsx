// yn — split from Overview.tsx; barrel re-exports keep every public path & name



/* ── helpers ─────────────────────────────────────────────────── */

export function yn(v: boolean | undefined): string {
  return v ? "y" : "n";
}
