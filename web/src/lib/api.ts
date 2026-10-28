/** Typed, bounded admin transport. Tokens are memory-only. GET retries are
 * one-shot; POST/download mutations are never automatically replayed.
 * Deadlines include consuming the response body, not just receiving headers.
 */
export const DEFAULT_TIMEOUT_MS = 15_000;
const PROBE_TIMEOUT_MS = 4_000;
const RETRY_DELAY_MS = 300;
const MAX_JSON_BYTES = 16 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES = 256 * 1024 * 1024;
const CACHE_MS = 2_000;
const CACHE_CAP = 64;

export type ApiErrorKind =
  | "network" | "timeout" | "aborted" | "unauthorized" | "notfound"
  | "method" | "payload" | "slowdown" | "conflict" | "server";

export class ApiError extends Error {
  readonly status: number | null;
  readonly kind: ApiErrorKind;
  readonly code: string | null;
  readonly requestId: string | null;
  constructor(kind: ApiErrorKind, message: string, opts: { status?: number; code?: string; requestId?: string } = {}) {
    super(message);
    this.name = "ApiError";
    this.kind = kind;
    this.status = opts.status ?? null;
    this.code = opts.code ?? null;
    this.requestId = opts.requestId ?? null;
  }
}

/** Persist only an endpoint with no userinfo, query, fragment or implicit
 * protocol. A URL-shaped preference must not become a secret store. */
function normalizeBase(base: string): string {
  const value = base.trim();
  if (!value) return "";
  const invalid = () => new ApiError("payload", "API endpoint must be a relative path or an http(s) URL without credentials, query or fragment");
  if (/[\s\\?#]/.test(value) || value.startsWith("//")) throw invalid();
  if (value.startsWith("/")) return value.replace(/\/+$/, "");
  let parsed: URL;
  try { parsed = new URL(value); } catch { throw invalid(); }
  if (!/^https?:$/.test(parsed.protocol) || parsed.username || parsed.password || parsed.search || parsed.hash) throw invalid();
  return parsed.href.replace(/\/+$/, "");
}
function envBase(): string {
  return normalizeBase((import.meta.env.VITE_API_BASE as string | undefined) ?? (import.meta.env.DEV ? "/api" : ""));
}
let runtimeBase: string | null = null;
try {
  const saved = globalThis.localStorage?.getItem("harvest.console.api");
  if (saved) runtimeBase = normalizeBase(saved);
} catch {
  try { globalThis.localStorage?.removeItem("harvest.console.api"); } catch { /* optional storage */ }
}
export const OPTS = { get BASE(): string { return runtimeBase ?? envBase(); } };

let bearer: string | null = null;
const authListeners = new Set<(set: boolean) => void>();
const active = new Set<AbortController>();
let generation = 0;
const cache = new Map<string, { at: number; data: unknown }>();
const inflight = new Map<string, Promise<unknown>>();

export function invalidateCache(prefix?: string): void {
  const matches = (key: string) => !prefix || key.startsWith(prefix) || key.startsWith(`${OPTS.BASE}${prefix}`);
  for (const key of cache.keys()) if (matches(key)) cache.delete(key);
  for (const key of inflight.keys()) if (matches(key)) inflight.delete(key);
  generation += 1; // invalidated requests may finish, but cannot refill stale cache
}
function resetConnection(): void {
  invalidateCache();
  for (const ctrl of active) ctrl.abort();
}
function normalizeToken(token: string | null): string | null {
  const value = token?.trim() || null;
  if (value && (/\s/.test(value) || [...value].some((c) => c.charCodeAt(0) < 0x20 || c.charCodeAt(0) === 0x7f))) throw new ApiError("payload", "Bearer token must not contain whitespace or control characters");
  return value;
}
export const auth = {
  set(token: string | null): void {
    const value = normalizeToken(token);
    if (value !== bearer) { bearer = value; resetConnection(); }
    authListeners.forEach((fn) => fn(bearer !== null));
  },
  get(): string | null { return bearer; },
  has(): boolean { return bearer !== null; },
  subscribe(fn: (set: boolean) => void): () => void {
    authListeners.add(fn);
    return () => authListeners.delete(fn);
  },
};
export function setApiBase(base: string | null, persist = true): void {
  const next = base && base.trim() ? normalizeBase(base) : null;
  const changed = (next ?? envBase()) !== OPTS.BASE;
  runtimeBase = next;
  if (changed) auth.set(null); // never send an old engine's token to a new endpoint
  resetConnection();
  if (persist) {
    try {
      if (next !== null) localStorage.setItem("harvest.console.api", next || "/");
      else localStorage.removeItem("harvest.console.api");
    } catch { /* persistence is optional */ }
  }
}
function headers(): Record<string, string> {
  return { accept: "application/json", ...(bearer ? { authorization: `Bearer ${bearer}` } : {}) };
}

interface ErrBody { error?: { code?: string; message?: string }; request_id?: string }
function networkError(e: unknown): ApiError {
  return e instanceof ApiError ? e : new ApiError("network", "admin API unreachable");
}

async function boundedBytes(res: Response, limit: number, signal?: AbortSignal): Promise<Uint8Array> {
  const declared = res.headers.get("content-length");
  if (declared && Number(declared) > limit) {
    // A rejected body may otherwise continue downloading in the background.
    void res.body?.cancel().catch(() => undefined);
    throw new ApiError("payload", "admin response exceeds the client size limit");
  }
  if (!res.body) return new Uint8Array();
  const reader = res.body.getReader();
  const chunks: Uint8Array[] = [];
  const cancel = () => { void reader.cancel().catch(() => undefined); };
  signal?.addEventListener("abort", cancel, { once: true });
  // AbortSignal does not replay an abort event for listeners attached after
  // cancellation; handle that ordering explicitly to avoid a dangling reader.
  if (signal?.aborted) cancel();
  let total = 0;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > limit) {
        void reader.cancel().catch(() => undefined);
        throw new ApiError("payload", "admin response exceeds the client size limit");
      }
      chunks.push(value);
    }
  } finally { signal?.removeEventListener("abort", cancel); reader.releaseLock(); }
  const out = new Uint8Array(total);
  let at = 0;
  for (const chunk of chunks) { out.set(chunk, at); at += chunk.byteLength; }
  return out;
}
async function toApiError(res: Response, token: string | null = bearer, signal?: AbortSignal): Promise<ApiError> {
  const text = new TextDecoder().decode(await boundedBytes(res, MAX_JSON_BYTES, signal));
  let body: ErrBody | null = null;
  try { body = JSON.parse(text) as ErrBody; } catch { /* plain error response */ }
  const candidate = typeof body?.error?.message === "string" ? body.error.message : text;
  let message = candidate;
  for (const secret of [token, bearer]) if (secret) message = message.split(secret).join("***");
  message = message.slice(0, 200);
  const kind: ApiErrorKind =
    res.status === 401 || res.status === 403 ? "unauthorized" : res.status === 404 ? "notfound" :
      res.status === 405 ? "method" : res.status === 400 || res.status === 413 ? "payload" :
        res.status === 429 ? "slowdown" : res.status === 409 ? "conflict" : "server";
  return new ApiError(kind, message || `HTTP ${res.status}`, {
    status: res.status,
    code: typeof body?.error?.code === "string" ? body.error.code : undefined,
    requestId: typeof body?.request_id === "string" ? body.request_id : undefined,
  });
}
async function readJson<T>(res: Response, token: string | null = bearer, signal?: AbortSignal): Promise<T> {
  if (!res.ok) throw await toApiError(res, token, signal);
  const text = new TextDecoder().decode(await boundedBytes(res, MAX_JSON_BYTES, signal));
  try { return JSON.parse(text) as T; }
  catch { throw new ApiError("server", "admin API returned invalid JSON", { status: res.status }); }
}

/** A hard deadline encloses fetch AND body consumption. Promise.race also
 * bounds a misbehaving adapter that ignores AbortSignal; native fetch is
 * aborted, and signal listeners/controllers are always released. */
async function timedRequest<T>(input: string, init: RequestInit, ms: number, consume: (res: Response, token: string | null, signal: AbortSignal) => Promise<T>): Promise<T> {
  const ctrl = new AbortController();
  const outer = init.signal;
  const authorization = new Headers(init.headers).get("authorization");
  const token = authorization?.startsWith("Bearer ") ? authorization.slice(7) : null;
  let timedOut = false;
  let rejectAbort: (error: ApiError) => void = () => undefined;
  const aborted = new Promise<never>((_resolve, reject) => { rejectAbort = reject; });
  const onAbort = () => rejectAbort(new ApiError(timedOut ? "timeout" : "aborted", timedOut ? `request timed out after ${ms} ms` : "request superseded"));
  const onOuter = () => ctrl.abort();
  ctrl.signal.addEventListener("abort", onAbort, { once: true });
  outer?.addEventListener("abort", onOuter, { once: true });
  active.add(ctrl);
  const timer = setTimeout(() => { timedOut = true; ctrl.abort(); }, ms);
  try {
    if (outer?.aborted) throw new ApiError("aborted", "request superseded");
    return await Promise.race([
      fetch(input, { ...init, credentials: "omit", redirect: "error", signal: ctrl.signal }).then((res) => consume(res, token, ctrl.signal)),
      aborted,
    ]);
  } catch (e) { throw networkError(e); }
  finally {
    clearTimeout(timer);
    outer?.removeEventListener("abort", onOuter);
    ctrl.signal.removeEventListener("abort", onAbort);
    active.delete(ctrl);
  }
}
const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
export interface RequestOpts {
  params?: Record<string, string | number | undefined>;
  signal?: AbortSignal;
  noCache?: boolean;
  timeoutMs?: number;
}
function url(path: string, params?: RequestOpts["params"]): string {
  const u = `${OPTS.BASE}${path}`;
  if (!params) return u;
  const qs = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) if (value !== undefined) qs.set(key, String(value));
  const suffix = qs.toString();
  return suffix ? `${u}${u.includes("?") ? "&" : "?"}${suffix}` : u;
}
export async function apiGet<T>(path: string, opts: RequestOpts = {}): Promise<T> {
  if (opts.signal?.aborted) throw new ApiError("aborted", "request superseded");
  const key = url(path, opts.params);
  // Caller-owned abort signals must not be shared: aborting A cannot kill B.
  const share = !opts.noCache && !opts.signal;
  const epoch = generation;
  const requestHeaders = headers(); // immutable auth snapshot for this endpoint
  if (share) {
    for (const [k, value] of cache) if (Date.now() - value.at >= CACHE_MS) cache.delete(k);
    const hit = cache.get(key);
    if (hit) return hit.data as T;
    const flying = inflight.get(key) as Promise<T> | undefined;
    if (flying) return flying;
  }
  const once = () => {
    if (epoch !== generation) throw new ApiError("aborted", "request superseded");
    return timedRequest(key, { headers: requestHeaders, signal: opts.signal }, opts.timeoutMs ?? DEFAULT_TIMEOUT_MS, readJson<T>);
  };
  const remember = (data: T) => {
    if (share && generation === epoch && (JSON.stringify(data)?.length ?? 0) <= 65_536) {
      while (cache.size >= CACHE_CAP) { const oldest = cache.keys().next().value; if (oldest === undefined) break; cache.delete(oldest); }
      cache.set(key, { at: Date.now(), data });
    }
    return data;
  };
  const task = (async () => {
    try { return remember(await once()); }
    catch (e) {
      const ae = networkError(e);
      if ((ae.kind === "network" || ae.kind === "timeout" || ae.status === 503) && !opts.signal?.aborted && epoch === generation) {
        await sleep(RETRY_DELAY_MS);
        if (opts.signal?.aborted || epoch !== generation) throw new ApiError("aborted", "request superseded");
        return remember(await once());
      }
      throw ae;
    }
  })();
  if (share) {
    inflight.set(key, task);
    const release = () => { if (inflight.get(key) === task) inflight.delete(key); };
    void task.then(release, release); // an old request must not delete a newer flight
  }
  return task;
}

/** POST is never retried. Server idempotency keys are 1..64 ASCII characters
 * of letters/digits/_/-. Replaying the same key/body returns the original job. */
export async function apiPost<T>(path: string, body: unknown, opts: { signal?: AbortSignal; idempotencyKey?: string } = {}): Promise<T> {
  return timedRequest(url(path), {
    method: "POST", headers: { ...headers(), "content-type": "application/json", ...(opts.idempotencyKey ? { "idempotency-key": opts.idempotencyKey } : {}) },
    body: JSON.stringify(body), signal: opts.signal,
  }, DEFAULT_TIMEOUT_MS, readJson<T>);
}
export function newCommandIdempotencyNonce(): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.getRandomValues === "function") {
    const bytes = new Uint8Array(9);
    cryptoApi.getRandomValues(bytes);
    return [...bytes].map((byte) => byte.toString(36).padStart(2, "0")).join("").slice(0, 16);
  }
  return `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 12)}`;
}

/**
 * Derive a repeatable key for one draft + nonce. The caller keeps the nonce
 * until a POST succeeds, so network retries and quick repeated events reuse
 * the same key; a new nonce is issued after acceptance for an intentional
 * later submission of the same command.
 */
export function commandIdempotencyKey(op: string, body: unknown, nonce: string): string {
  const fingerprint = JSON.stringify(body ?? null);
  let hash = 0x811c9dc5;
  for (let i = 0; i < fingerprint.length; i += 1) { hash ^= fingerprint.charCodeAt(i); hash = Math.imul(hash, 0x01000193) >>> 0; }
  const safeOp = op.slice(0, 24).replace(/[^A-Za-z0-9_-]/g, "_");
  const safeNonce = nonce.replace(/[^A-Za-z0-9_-]/g, "_").slice(0, 20) || "retry";
  return `${safeOp}-${hash.toString(36)}-${safeNonce}`;
}
export interface ArtifactDownload {
  bytes: Uint8Array;
  contentType: string | null;
  sha256: string | null;
  artifactId: string | null;
  state: string | null;
  filename: string | null;
}
export async function apiDownload(path: string, opts: { signal?: AbortSignal; timeoutMs?: number } = {}): Promise<ArtifactDownload> {
  return timedRequest(url(path), { headers: { ...headers(), accept: "*/*" }, signal: opts.signal }, opts.timeoutMs ?? DEFAULT_TIMEOUT_MS, async (res, token, signal) => {
    if (!res.ok) throw await toApiError(res, token, signal);
    const bytes = await boundedBytes(res, MAX_DOWNLOAD_BYTES, signal);
    const match = /filename="([^"\\/\r\n]+)"/.exec(res.headers.get("content-disposition") ?? "");
    return { bytes, contentType: res.headers.get("content-type"), sha256: res.headers.get("x-artifact-sha256"), artifactId: res.headers.get("x-artifact-id"), state: res.headers.get("x-artifact-state"), filename: match?.[1] ?? null };
  });
}
export function saveDownload(dl: ArtifactDownload, fallbackName = "artifact.bin"): string | null {
  const name = dl.filename ?? fallbackName;
  const create = globalThis.URL?.createObjectURL;
  if (typeof create !== "function") return null;
  const copy = new Uint8Array(dl.bytes.byteLength);
  copy.set(dl.bytes);
  const href = create.call(globalThis.URL, new Blob([copy], { type: dl.contentType ?? "application/octet-stream" }));
  try {
    const a = document.createElement("a");
    a.href = href; a.download = name; a.rel = "noopener";
    document.body.appendChild(a); a.click(); a.remove();
  } finally { setTimeout(() => globalThis.URL?.revokeObjectURL?.(href), 0); }
  return name;
}
export async function fetchMetricsText(opts: RequestOpts = {}): Promise<string> {
  return timedRequest(url("/metrics"), { headers: headers(), signal: opts.signal }, opts.timeoutMs ?? DEFAULT_TIMEOUT_MS, async (res, token, signal) => {
    if (!res.ok) throw await toApiError(res, token, signal);
    return new TextDecoder().decode(await boundedBytes(res, MAX_JSON_BYTES, signal));
  });
}
export function promGauge(text: string, name: string): number | null {
  for (const line of text.split("\n")) if (line.startsWith(name + " ")) {
    const value = Number(line.slice(name.length + 1).trim().split(/\s+/)[0]);
    return Number.isFinite(value) ? value : null;
  }
  return null;
}
export interface ProbeResult { ok: boolean; latencyMs: number | null; error: ApiError | null }
/** /health is auth-exempt: also check /stats so an incorrect/missing token
 * cannot produce a misleading "connected" result. One deadline covers both. */
export async function probeHealth(base: string, token: string | null, timeoutMs = PROBE_TIMEOUT_MS): Promise<ProbeResult> {
  const start = Date.now();
  try {
    const endpoint = normalizeBase(base);
    const value = normalizeToken(token);
    const init = { headers: { accept: "application/json", ...(value ? { authorization: `Bearer ${value}` } : {}) } };
    const health = await timedRequest<{ status?: string }>(`${endpoint}/health`, init, timeoutMs, readJson);
    if (health?.status !== "ok") throw new ApiError("server", "endpoint did not return harvest health JSON");
    const remaining = timeoutMs - (Date.now() - start);
    if (remaining <= 0) throw new ApiError("timeout", `request timed out after ${timeoutMs} ms`);
    await timedRequest(`${endpoint}/stats`, init, remaining, readJson);
    return { ok: true, latencyMs: Date.now() - start, error: null };
  } catch (e) { return { ok: false, latencyMs: Date.now() - start, error: networkError(e) }; }
}
