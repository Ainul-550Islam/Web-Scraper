/** Number/time formatting used across the console. Tabular-safe output. */

export function fmtNum(n: number | null | undefined): string {
  if (n === null || n === undefined || Number.isNaN(n)) return "—";
  return n.toLocaleString("en-US", { maximumFractionDigits: 2 });
}

export function fmtRate(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return "—";
  return n < 10 ? n.toFixed(2) : n.toFixed(1);
}

export function fmtPct(fraction: number | null | undefined): string {
  if (fraction === null || fraction === undefined || !Number.isFinite(fraction))
    return "—";
  return `${(fraction * 100).toFixed(fraction > 0.0995 || fraction === 0 ? 0 : 1)}%`;
}

export function fmtMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return "—";
  if (ms < 1) return "<1 ms";
  if (ms < 1000) return `${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
  const h = Math.floor(ms / 3_600_000);
  const m = Math.floor((ms % 3_600_000) / 60_000);
  return `${h}h ${m}m`;
}

export function fmtBytes(b: number | null | undefined): string {
  if (b === null || b === undefined || !Number.isFinite(b)) return "—";
  if (b < 1024) return `${b} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let v = b / 1024;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[u]}`;
}

export function fmtClock(unixSec: number | null | undefined): string {
  if (!unixSec) return "—";
  const d = new Date(unixSec * 1000);
  return d.toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function fmtDateTime(unixSec: number | null | undefined): string {
  if (!unixSec) return "—";
  const d = new Date(unixSec * 1000);
  return d.toLocaleString("en-GB", {
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function fmtAgo(unixSec: number | null | undefined, nowSec?: number): string {
  if (!unixSec) return "—";
  const secs = Math.max(0, (nowSec ?? Math.floor(Date.now() / 1000)) - unixSec);
  if (secs < 5) return "just now";
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

/** "a1b2c3d4…90ef" style hash shortening for policy/config sha columns. */
export function fmtHash(sha: string | null | undefined, head = 8): string {
  if (!sha) return "—";
  if (sha.length <= head + 6) return sha;
  return `${sha.slice(0, head)}…${sha.slice(-4)}`;
}

export function fmtPath(p: string | null | undefined): string {
  if (!p) return "—";
  return p;
}

/** Mask anything that looks like credentials inside free-text values. */
export function maskSecrets(text: string): string {
  return text
    .replace(/\b(Bearer\s+)[A-Za-z0-9._~+/=-]+/gi, "$1<redacted>")
    .replace(/(https?:\/\/[^/\s:@]+:)[^@\s]+@/gi, "$1<redacted>@")
    .replace(
      /\b(admin_token|password|secret|token|api[_-]?key)("?\s*[:=]\s*"?)([^",\s}\]]+)/gi,
      "$1$2<redacted>",
    );
}
