// navigate — split from router.ts; barrel re-exports keep every public path & name



export function navigate(to: string): void {
  window.location.hash = to;
}
