// toneKind — split from Audit.tsx; barrel re-exports keep every public path & name



export function toneKind(k: string): string {
  if (k.includes("fail") || k.includes("error")) return "error";
  if (k.includes("start") || k.includes("ack") || k.includes("note")) return "info";
  if (k.includes("end") || k.includes("ok") || k.includes("complete")) return "ok";
  return "idle";
}
