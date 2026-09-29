/**
 * redaction — structural secret-masking for arbitrary JSON (responses,
 * event details, pasted configs) that `maskSecrets` (line-oriented text
 * masking in `format.ts`) was not designed for.
 *
 * Rules, mirroring the server's own policy (`GET /config` nulls secrets and
 * never sends plaintext):
 *  - any object key matching the credential vocabulary below is replaced with
 *    the literal "***" in the OUTPUT COPY — the input object is never mutated
 *    (pages poll shared objects; mutating them would corrupt the cache);
 *  - URL-shaped strings carrying inline `user:pass@host` credentials have the
 *    userinfo redacted;
 *  - recursion covers nested records and arrays to any depth.
 *
 * This is display-side defense in depth: the browser only ever *sees* what
 * the API sent, but pasted content and future endpoints make a client-side
 * pass worth having. It is not a security boundary.
 */

const SECRET_KEY_RE = /(token|authoriz|password|passwd|secret|webhook|api[_-]?key|dsn|credential)/i;
const CRED_URL_RE = /\/\/[^/@\s]+@/;

export const REDACTION_MARK = "***";

function redactUrlish(s: string): string {
  return CRED_URL_RE.test(s) ? s.replace(/\/\/([^/@\s]*)@/, `//${REDACTION_MARK}@`) : s;
}

export function deepMask<T>(value: T): T {
  return deepMaskInner(value) as T;
}

function deepMaskInner(value: unknown): unknown {
  if (typeof value === "string") return redactUrlish(value);
  if (Array.isArray(value)) return value.map(deepMaskInner);
  if (value !== null && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      out[k] = SECRET_KEY_RE.test(k) ? REDACTION_MARK : deepMaskInner(v);
    }
    return out;
  }
  return value;
}

/** True when `text` looks like it still contains a credential-bearing URL —
 *  used by tests and by the JSON inspector's copy guard. */
export function looksUnredacted(text: string): boolean {
  return CRED_URL_RE.test(text);
}
