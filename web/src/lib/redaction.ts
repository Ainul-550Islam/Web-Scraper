/** Display-side defense in depth; never mutates polled/shared objects. */
const SECRET_KEY_RE = /(token|authoriz|password|passwd|secret|webhook|api[_-]?key|access[_-]?key|private[_-]?key|dsn|credential|cookie)/i;
const CRED_URL_RE = /\/\/(?!\*\*\*@)[^/@\s]+@/;
export const REDACTION_MARK = "***";
export const MAX_MASK_NODES = 10_000;
const MAX_MASK_DEPTH = 64;

function redactUrlish(value: string): string {
  return value
    .replace(/[a-z][a-z0-9+.-]*:\/\/[^\s"<>]+/gi, (raw) => {
      try {
        const url = new URL(raw);
        let changed = false;
        if (url.username || url.password) {
          url.username = REDACTION_MARK;
          url.password = "";
          changed = true;
        }
        for (const key of [...url.searchParams.keys()]) {
          if (SECRET_KEY_RE.test(key) || key.toLowerCase() === "key") {
            url.searchParams.set(key, REDACTION_MARK);
            changed = true;
          }
        }
        return changed ? url.toString() : raw;
      } catch {
        return raw;
      }
    })
    .replace(/\/\/([^/@\s]*)@/g, `//${REDACTION_MARK}@`)
    .replace(/\bBearer\s+[^\s"<>]+/gi, `Bearer ${REDACTION_MARK}`)
    .replace(/([?&](?:token|access_token|api[_-]?key|key|password|secret)=)[^&#\s]*/gi, `$1${REDACTION_MARK}`);
}

interface MaskBudget {
  remaining: number;
  exhausted: boolean;
}

/**
 * Redact a JSON-like value without mutating it and with a hard traversal cap.
 * Once the budget is exhausted, the remainder is represented by `***`; this
 * keeps render/copy work bounded and never exposes unvisited values.
 */
export function deepMask<T>(value: T, maxNodes = MAX_MASK_NODES): T {
  const limit = Number.isFinite(maxNodes)
    ? Math.max(1, Math.min(MAX_MASK_NODES, Math.floor(maxNodes)))
    : MAX_MASK_NODES;
  const budget: MaskBudget = { remaining: limit, exhausted: false };
  return deepMaskInner(value, new WeakSet(), 0, budget) as T;
}

function defineSafe(target: Record<string, unknown>, key: string, value: unknown): void {
  Object.defineProperty(target, key, {
    value,
    enumerable: true,
    writable: true,
    configurable: true,
  });
}

function deepMaskInner(
  value: unknown,
  ancestors: WeakSet<object>,
  depth: number,
  budget: MaskBudget,
): unknown {
  if (budget.remaining <= 0) {
    budget.exhausted = true;
    return REDACTION_MARK;
  }
  budget.remaining -= 1;

  if (typeof value === "string") return redactUrlish(value);
  if (value === null || typeof value !== "object") return value;
  if (depth >= MAX_MASK_DEPTH || ancestors.has(value)) return REDACTION_MARK;

  ancestors.add(value);
  if (Array.isArray(value)) {
    const copy: unknown[] = [];
    for (let index = 0; index < value.length; index += 1) {
      if (budget.remaining <= 0) {
        copy.push(REDACTION_MARK);
        budget.exhausted = true;
        break;
      }
      copy.push(deepMaskInner(value[index], ancestors, depth + 1, budget));
      if (budget.exhausted) break;
    }
    ancestors.delete(value);
    return copy;
  }

  const copy: Record<string, unknown> = {};
  for (const key in value) {
    if (!Object.hasOwn(value, key)) continue;
    if (budget.remaining <= 0) {
      defineSafe(copy, "__truncated", REDACTION_MARK);
      budget.exhausted = true;
      break;
    }
    const item = (value as Record<string, unknown>)[key];
    if (SECRET_KEY_RE.test(key)) {
      budget.remaining -= 1;
      defineSafe(copy, key, REDACTION_MARK);
    } else {
      defineSafe(copy, key, deepMaskInner(item, ancestors, depth + 1, budget));
    }
    if (budget.exhausted) break;
  }
  ancestors.delete(value);
  return copy;
}

export function looksUnredacted(text: string): boolean {
  return CRED_URL_RE.test(text) || /[?&](?:token|api[_-]?key|access_token|password|secret)=(?!\*\*\*|%2A%2A%2A)[^&#\s]+/i.test(text);
}
