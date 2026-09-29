// GOTO — split from useShortcuts.ts; barrel re-exports keep every public path & name


export const GOTO: Readonly<Record<string, string>> = {
  o: "/",
  r: "/runs",
  j: "/jobs",
  w: "/workers",
  p: "/proxies",
  q: "/queue",
  c: "/config",
};
