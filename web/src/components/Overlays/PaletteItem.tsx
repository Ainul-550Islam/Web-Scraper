// PaletteItem — split from Overlays.tsx; barrel re-exports keep every public path & name



/* ── command palette: navigation + actions + record search ───── */
export interface PaletteItem {
  id: string;
  group: "Navigate" | "Actions" | "Records";
  label: string;
  hint?: string;
  run: () => void;
}
