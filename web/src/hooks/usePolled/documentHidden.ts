// documentHidden — split from usePolled.ts; barrel re-exports keep every public path & name



export function documentHidden(): boolean {
  return typeof document !== "undefined" && document.visibilityState === "hidden";
}
