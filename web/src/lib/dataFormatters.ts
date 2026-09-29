/**
 * dataFormatters — the presentation-side helpers the new operational pages
 * share, layered on top of (never replacing) `src/lib/format.ts`. Everything
 * here is a pure function of its inputs: no fetching, no locale roulette
 * beyond what `format.ts` already standardizes, no invented fields.
 *
 * `format.ts` stays the single source for the established fmt* family and is
 * re-exported wholesale, so any page can import the whole vocabulary from
 * one module without breaking existing `../lib/format` imports elsewhere.
 */
export * from "./format";

/** Long-form duration for dashboards: 7_384_000 ms → "2h 03m 04s". */
export function humanDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) return "—";
  const s = Math.floor(ms / 1000);
  const parts: string[] = [];
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) parts.push(`${h}h`);
  if (h > 0 || m > 0) parts.push(`${String(m).padStart(h > 0 ? 2 : 1, "0")}m`);
  parts.push(`${String(sec).padStart(parts.length > 0 ? 2 : 1, "0")}s`);
  if (parts.length === 1 && ms < 1000) return `${ms}ms`;
  return parts.join(" ");
}

/** Human labels for the engine's run/lane statuses. Unknown values pass
 *  through verbatim — the console never hides a status it doesn't know. */
export function statusLabel(status: string | null | undefined): string {
  switch (status) {
    case null:
    case undefined:
      return "—";
    case "ok":
      return "Completed";
    case "error":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    case "running":
      return "Running";
    case "degraded":
      return "Degraded";
    case "disabled":
      return "Not configured";
    case "unavailable":
      return "Unavailable";
    default:
      return status;
  }
}

/** Unix seconds → ISO-8601 (UTC). Null-safe for absent timestamps. */
export function epochToIso(unixSec: number | null | undefined): string {
  if (unixSec === null || unixSec === undefined || !Number.isFinite(unixSec)) return "—";
  return new Date(unixSec * 1000).toISOString();
}

/** Percentage of a total, guarding division by zero the way every other
 *  console panel does (0 of 0 is 0%, never NaN). */
export function pctOf(part: number, total: number): number {
  if (!Number.isFinite(part) || !Number.isFinite(total) || total <= 0) return 0;
  return (part / total) * 100;
}

/** Signed integer delta for trend slots: 5 → "+5", -2 → "−2", 0 → "±0". */
export function signedDelta(n: number): string {
  if (!Number.isFinite(n)) return "—";
  if (n > 0) return `+${n}`;
  if (n < 0) return `−${Math.abs(n)}`;
  return "±0";
}
