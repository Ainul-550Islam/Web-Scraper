// isEditable — split from useShortcuts.ts; barrel re-exports keep every public path & name



export function isEditable(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return (
    t instanceof HTMLInputElement ||
    t instanceof HTMLTextAreaElement ||
    t instanceof HTMLSelectElement ||
    t.isContentEditable === true
  );
}
