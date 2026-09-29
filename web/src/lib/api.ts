/**
 * Typed client for the harvest admin API — hardened per the network-
 * resilience and security briefs.
 *
 * Security invariants (mirrors the backend's own model):
 *  - the bearer token lives ONLY in module memory — never localStorage,
 *    never the URL, never logged;
 *  - the token is attached to `/api` requests only while set, and is
 *    deliberately omitted from error messages, console output, and the
 *    "copy diagnostics" flow;
 *  - every request is abortable: a newer call supersedes the older one so
 *    stale responses can never paint over fresh data.
 *
 * Transport invariants:
 *  - every fetch carries a timeout (default 15 s, 4 s for connection probes)
 *    so a wedged socket can never pin a spinner;
 *  - idempotent GETs retry exactly once, only for transient transport
 *    failures (network drop, 503); mutations (apiPost) are NEVER retried;
 *  - identical in-flight GETs share one request (single-flight) and short
 *    successful results share a 2 s TTL cache — polling storms are
 *    impossible by construction;
 *  - all failures surface as a normalized ApiError (kind + status +
 *    server code + request id), never as raw fetch rejections.
 */

export const DEFAULT_TIMEOUT_MS = 15_000;
const PROBE_TIMEOUT_MS = 4_000;
const RETRY_DELAY_MS = 300;

/* ── api base: env default, runtime override from the connect screen ── */
function envBase(): string {
  return (
    (import.meta.env.VITE_API_BASE as string | undefined) ??
    (import.meta.env.DEV ? "/api" : "")
  );
}

/**
 * Only the endpoint URL is persisted (`harvest.console.api`) — it is a
 * non-sensitive preference. Credentials never touch storage (§32).
 */
let runtimeBase: string | null = null;
try {
  const saved = globalThis.localStorage?.getItem("harvest.console.api");
  if (saved) runtimeBase = saved;
} catch {
  /* storage unavailable (node tests without jsdom, privacy modes) */
}

export const OPTS = {
  get BASE(): string {
    return runtimeBase ?? envBase();
  },
};

export function setApiBase(base: string | null, persist = true): void {
  runtimeBase = base && base.trim().length > 0 ? base.trim() : null;
  if (persist) {
    try {
      if (runtimeBase !== null) localStorage.setItem("harvest.console.api", runtimeBase);
      else localStorage.removeItem("harvest.console.api");
    } catch {
      /* persistence is a convenience, not a requirement */
    }
  }
  invalidateCache();
}

/* ── normalized errors (§33) ───────────────────────────────────── */
export type ApiErrorKind =
  | "network"
  | "timeout"
  | "aborted"
  | "unauthorized"
  | "notfound"
  | "method"
  | "payload"
  | "slowdown"
  /** 409 — a state conflict: cancelling a job that already finished, or
   *  reusing an Idempotency-Key with a different body. */
  | "conflict"
  | "server";

export class ApiError extends Error {
  readonly status: number | null;
  readonly kind: ApiErrorKind;
  readonly code: string | null;
  readonly requestId: string | null;

  constructor(
    kind: ApiErrorKind,
    message: string,
    opts: { status?: number; code?: string; requestId?: string } = {},
  ) {
    super(message);
    this.name = "ApiError";
    this.kind = kind;
    this.status = opts.status ?? null;
    this.code = opts.code ?? null;
    this.requestId = opts.requestId ?? null;
  }
}

/* ── in-memory auth store ─────────────────────────────────────── */
let bearer: string | null = null;
const authListeners = new Set<(set: boolean) => void>();

export const auth = {
  set(token: string | null): void {
    bearer = token && token.length > 0 ? token : null;
    authListeners.forEach((fn) => fn(bearer !== null));
  },
  get(): string | null {
    return bearer;
  },
  has(): boolean {
    return bearer !== null;
  },
  subscribe(fn: (set: boolean) => void): () => void {
    authListeners.add(fn);
    return () => authListeners.delete(fn);
  },
};

/* ── tiny safe GET cache (single-flight + short TTL) ──────────── */
const cache = new Map<string, { at: number; data: unknown }>();
const CACHE_MS = 2_000;
const inflight = new Map<string, Promise<unknown>>();

export function invalidateCache(prefix?: string): void {
  for (const k of [...cache.keys()]) {
    if (!prefix || k.startsWith(prefix)) cache.delete(k);
  }
  // shared single-flight promises are connection state too — dropping a
  // wedge (test stub, endpoint switch) must not strand future callers on a
  // promise that can never settle
  for (const k of [...inflight.keys()]) {
    if (!prefix || k.startsWith(prefix)) inflight.delete(k);
  }
}

function headers(): Record<string, string> {
  const h: Record<string, string> = { accept: "application/json" };
  if (bearer !== null) h.authorization = `Bearer ${bearer}`;
  return h;
}

interface ErrBody {
  error?: { code?: string; message?: string };
  request_id?: string;
}

async function toApiError(res: Response): Promise<ApiError> {
  let body: ErrBody | null = null;
  let text = "";
  try {
    text = await res.text();
    body = text.startsWith("{") ? (JSON.parse(text) as ErrBody) : null;
  } catch {
    body = null;
  }
  const message =
    body?.error?.message ??
    (res.status === 404 && text.includes("see /")
      ? "route not found"
      : text.slice(0, 200)) ??
    res.statusText;
  const kind: ApiErrorKind =
    res.status === 401 || res.status === 403
      ? "unauthorized"
      : res.status === 404
        ? "notfound"
        : res.status === 405
          ? "method"
          : res.status === 413 || res.status === 400
            ? "payload"
            : res.status === 429
              ? "slowdown"
              : res.status === 409
                ? "conflict"
                : "server";
  return new ApiError(kind, message || `HTTP ${res.status}`, {
    status: res.status,
    code: body?.error?.code ?? undefined,
    requestId: body?.request_id ?? undefined,
  });
}

/** fetch + timeout + external-signal chaining; throws ApiError itself. */
async function timedFetch(
  input: string,
  init: RequestInit,
  ms: number,
): Promise<Response> {
  const ctrl = new AbortController();
  let timedOut = false;
  const outer = init.signal ?? undefined;
  const onOuter = () => ctrl.abort();
  if (outer) {
    if (outer.aborted) ctrl.abort();
    else outer.addEventListener("abort", onOuter, { once: true });
  }
  const timer = setTimeout(() => {
    timedOut = true;
    ctrl.abort();
  }, ms);
  try {
    return await fetch(input, { ...init, signal: ctrl.signal });
  } catch (e) {
    if (timedOut) throw new ApiError("timeout", `request timed out after ${ms} ms`);
    if (outer?.aborted) throw new ApiError("aborted", "request superseded");
    if (e instanceof DOMException && e.name === "AbortError")
      throw new ApiError("aborted", "request superseded");
    throw e; // raw network failure — normalized by callers
  } finally {
    clearTimeout(timer);
    outer?.removeEventListener("abort", onOuter);
  }
}

function networkError(e: unknown): ApiError {
  return e instanceof ApiError
    ? e
    : new ApiError("network", "admin API unreachable");
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export interface RequestOpts {
  params?: Record<string, string | number | undefined>;
  signal?: AbortSignal;
  /** skip the short safe-GET cache (pollers use default; mutations never cache) */
  noCache?: boolean;
  /** override the 15 s transport timeout (probes use ~4 s) */
  timeoutMs?: number;
}

function url(path: string, params?: RequestOpts["params"]): string {
  const u = `${OPTS.BASE}${path}`;
  if (!params) return u;
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined) qs.set(k, String(v));
  }
  const s = qs.toString();
  return s ? `${u}?${s}` : u;
}

export async function apiGet<T>(path: string, opts: RequestOpts = {}): Promise<T> {
  const key = url(path, opts.params);
  if (!opts.noCache) {
    const hit = cache.get(key);
    if (hit && Date.now() - hit.at < CACHE_MS) return hit.data as T;
    const flying = inflight.get(key) as Promise<T> | undefined;
    if (flying) return flying; // identical concurrent GETs share one request
  }
  const timeoutMs = opts.timeoutMs ?? DEFAULT_TIMEOUT_MS;

  const once = async (): Promise<T> => {
    try {
      const res = await timedFetch(key, { headers: headers(), signal: opts.signal }, timeoutMs);
      if (!res.ok) throw await toApiError(res);
      return (await res.json()) as T;
    } catch (e) {
      throw networkError(e);
    }
  };

  const task = (async () => {
    try {
      const data = await once();
      if (!opts.noCache) cache.set(key, { at: Date.now(), data });
      return data;
    } catch (e) {
      const ae = e as ApiError;
      // §10: retry only idempotent reads, exactly once, and only for
      // transient transport failures. 429/5xx-json bodies are server
      // verdicts — retrying them here would hammer a degraded engine.
      const retryable = ae.kind === "network" || ae.kind === "timeout" || ae.status === 503;
      if (retryable && !opts.signal?.aborted) {
        await sleep(RETRY_DELAY_MS);
        if (opts.signal?.aborted) throw new ApiError("aborted", "request superseded");
        const data = await once();
        if (!opts.noCache) cache.set(key, { at: Date.now(), data });
        return data;
      }
      throw ae;
    } finally {
      inflight.delete(key);
    }
  })();

  if (!opts.noCache) inflight.set(key, task);
  return task;
}

/**
 * Mutations. Deliberately NOT retried by the transport: the client cannot
 * know whether a POST was processed before the socket died.
 *
 * Safety for a retried submission comes from the *server*, not from a client
 * retry: pass `idempotencyKey` and the control plane returns the original job
 * (HTTP 200 + `replayed: true`) instead of starting a second one, and answers
 * 409 when the same key arrives with a different body. The key is 8-128 chars
 * of `[A-Za-z0-9_-]` (the server's rule, mirrored by
 * `commandIdempotencyKey()`), and it leaves this module only as a header.
 */
export async function apiPost<T>(
  path: string,
  body: unknown,
  opts: { signal?: AbortSignal; idempotencyKey?: string } = {},
): Promise<T> {
  const extra: Record<string, string> = {};
  if (opts.idempotencyKey) extra["idempotency-key"] = opts.idempotencyKey;
  let res: Response;
  try {
    res = await timedFetch(
      url(path),
      {
        method: "POST",
        headers: { ...headers(), "content-type": "application/json", ...extra },
        body: JSON.stringify(body),
        signal: opts.signal,
      },
      DEFAULT_TIMEOUT_MS,
    );
  } catch (e) {
    throw networkError(e);
  }
  if (!res.ok) throw await toApiError(res);
  return (await res.json()) as T;
}

/**
 * An idempotency key the control plane accepts: stable for one command, so a
 * double-click (or a retry after a dropped connection) maps onto the same job
 * instead of starting a second one.
 */
export function commandIdempotencyKey(op: string, body: unknown): string {
  const fingerprint = JSON.stringify(body ?? null);
  let hash = 0x811c9dc5;
  for (let i = 0; i < fingerprint.length; i += 1) {
    hash ^= fingerprint.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `${op}-${hash.toString(36)}-${Date.now().toString(36)}`;
}

/** One artifact download: bytes plus the headers the server vouches for. */
export interface ArtifactDownload {
  bytes: Uint8Array;
  contentType: string | null;
  /** sha256 the registry recorded — compare before trusting the bytes. */
  sha256: string | null;
  artifactId: string | null;
  state: string | null;
  /** `filename="…"` parsed out of content-disposition; never a path. */
  filename: string | null;
}

/**
 * `GET /artifacts/:id` — the bytes. Authentication travels in a header, never
 * in the query string, so the token cannot end up in a URL, a referer or an
 * access log. The server refuses a delivery that failed verification (409) and
 * an id that is not a registered artifact (404); both surface as ApiError.
 */
export async function apiDownload(
  path: string,
  opts: { signal?: AbortSignal; timeoutMs?: number } = {},
): Promise<ArtifactDownload> {
  let res: Response;
  try {
    res = await timedFetch(
      url(path),
      { headers: { ...headers(), accept: "*/*" }, signal: opts.signal },
      opts.timeoutMs ?? DEFAULT_TIMEOUT_MS,
    );
  } catch (e) {
    throw networkError(e);
  }
  if (!res.ok) throw await toApiError(res);
  const bytes = new Uint8Array(await res.arrayBuffer());
  const cd = res.headers.get("content-disposition") ?? "";
  const m = /filename="([^"]+)"/.exec(cd);
  const head = (k: string): string | null => res.headers.get(k) ?? null;
  return {
    bytes,
    contentType: head("content-type"),
    sha256: head("x-artifact-sha256"),
    artifactId: head("x-artifact-id"),
    state: head("x-artifact-state"),
    filename: m ? (m[1] ?? null) : null,
  };
}

/**
 * Hand bytes to the browser as a file download. The object URL is revoked on
 * the next tick; where `createObjectURL` is unavailable (tests, exotic
 * runtimes) this is a no-op instead of a crash.
 */
export function saveDownload(dl: ArtifactDownload, fallbackName = "artifact.bin"): string | null {
  const name = dl.filename ?? fallbackName;
  const create = globalThis.URL?.createObjectURL;
  if (typeof create !== "function") return null;
  // copy into a fresh ArrayBuffer: `Uint8Array<ArrayBufferLike>` (which is
  // what a Response body can yield) is not a `BlobPart` under strict TS
  const copy = new Uint8Array(dl.bytes.byteLength);
  copy.set(dl.bytes);
  const blob = new Blob([copy], { type: dl.contentType ?? "application/octet-stream" });
  const href = create.call(globalThis.URL, blob);
  try {
    const a = document.createElement("a");
    a.href = href;
    a.download = name;
    a.rel = "noopener";
    document.body.appendChild(a);
    a.click();
    a.remove();
  } finally {
    setTimeout(() => {
      globalThis.URL?.revokeObjectURL?.(href);
    }, 0);
  }
  return name;
}

/** Plain-text scrape of /metrics (Prometheus exposition). */
export async function fetchMetricsText(opts: RequestOpts = {}): Promise<string> {
  try {
    const res = await timedFetch(
      url("/metrics"),
      { headers: headers(), signal: opts.signal },
      opts.timeoutMs ?? DEFAULT_TIMEOUT_MS,
    );
    if (!res.ok) throw await toApiError(res);
    return await res.text();
  } catch (e) {
    throw networkError(e);
  }
}

/** Parse `harvest_<name> <value>` lines from the Prometheus text. */
export function promGauge(text: string, name: string): number | null {
  for (const line of text.split("\n")) {
    if (line.startsWith(name + " ")) return Number(line.slice(name.length + 1));
  }
  return null;
}

/* ── connection probe (used by the first-run connect screen) ───── */
export interface ProbeResult {
  ok: boolean;
  latencyMs: number | null;
  error: ApiError | null;
}

/**
 * One-shot, side-effect-free health probe against an arbitrary base+token.
 * Never throws; never logs; the token argument is used for exactly one
 * header and kept nowhere.
 */
export async function probeHealth(
  base: string,
  token: string | null,
  timeoutMs = PROBE_TIMEOUT_MS,
): Promise<ProbeResult> {
  const t0 = Date.now();
  try {
    const res = await timedFetch(
      `${base}/health`,
      {
        headers: {
          accept: "application/json",
          ...(token ? { authorization: `Bearer ${token}` } : {}),
        },
      },
      timeoutMs,
    );
    if (!res.ok) throw await toApiError(res);
    await res.json();
    return { ok: true, latencyMs: Date.now() - t0, error: null };
  } catch (e) {
    return {
      ok: false,
      latencyMs: Date.now() - t0,
      error: networkError(e),
    };
  }
}
