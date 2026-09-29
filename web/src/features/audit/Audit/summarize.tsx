// summarize — split from Audit.tsx; barrel re-exports keep every public path & name



export function summarize(d: Record<string, unknown>): string {
  const parts: string[] = [];
  if (d["run"]) parts.push(`run ${String(d["run"])}`);
  if (d["event"]) parts.push(`event ${String(d["event"])}`);
  if (d["written"] !== undefined) parts.push(`written ${String(d["written"])}`);
  if (d["note"]) parts.push(String(d["note"]).slice(0, 60));
  if (d["status"]) parts.push(`status ${String(d["status"])}`);
  if (d["entries"] !== undefined) parts.push(`${String(d["entries"])} entries`);
  return parts.join(" · ") || "—";
}
