import { afterEach, describe, expect, it, vi } from "vitest";
import { apiDownload, apiGet, auth, invalidateCache, probeHealth, setApiBase } from "../../lib/api";
import { deepMask } from "../../lib/redaction";
import { jsonResponse } from "./helpers";
afterEach(() => { auth.set(null); setApiBase(null, false); invalidateCache(); });
describe("review regressions", () => {
  it("rejects secret-bearing and non-http endpoint preferences before persisting", () => {
    localStorage.removeItem("harvest.console.api");
    for (const endpoint of ["https://u:p@host/api", "https://host/api?token=secret", "https://host/#token", "//host/api", "javascript:alert(1)"]) {
      expect(() => setApiBase(endpoint)).toThrow();
      expect(localStorage.getItem("harvest.console.api")).toBeNull();
    }
  });
  it("changing auth invalidates privileged cached GETs", async () => {
    const f = vi.fn(async () => jsonResponse({ n: f.mock.calls.length }));
    vi.stubGlobal("fetch", f);
    auth.set("old-token");
    await apiGet("/audit-auth");
    auth.set("new-token");
    const second = await apiGet<{ n: number }>("/audit-auth");
    expect(second.n).toBe(2);
  });
  it("an old in-flight completion cannot replace the new cache or delete its request", async () => {
    const replies: Array<(r: Response) => void> = [];
    const f = vi.fn(() => new Promise<Response>((resolve) => replies.push(resolve)));
    vi.stubGlobal("fetch", f);
    const old = apiGet("/audit-generation");
    invalidateCache();
    const fresh = apiGet("/audit-generation");
    replies[0]!(jsonResponse({ generation: "old" }));
    await old;
    const follower = apiGet("/audit-generation");
    expect(f).toHaveBeenCalledTimes(2);
    replies[1]!(jsonResponse({ generation: "fresh" }));
    await expect(fresh).resolves.toEqual({ generation: "fresh" });
    await expect(follower).resolves.toEqual({ generation: "fresh" });
    await expect(apiGet("/audit-generation")).resolves.toEqual({ generation: "fresh" });
  });
  it("callers with independent AbortSignals cannot cancel each other", async () => {
    const f = vi.fn((_url: RequestInfo | URL, init?: RequestInit) => new Promise<Response>((resolve, reject) => {
      init?.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
      setTimeout(() => resolve(jsonResponse({ ok: true })), 20);
    }));
    vi.stubGlobal("fetch", f);
    const a = new AbortController();
    const b = new AbortController();
    const first = apiGet("/audit-independent", { signal: a.signal });
    const second = apiGet("/audit-independent", { signal: b.signal });
    a.abort();
    await expect(first).rejects.toMatchObject({ kind: "aborted" });
    await expect(second).resolves.toEqual({ ok: true });
    expect(f).toHaveBeenCalledTimes(2);
  });
  it("download deadlines cover a stalled body, even after HTTP headers arrived", async () => {
    const cancel = vi.fn();
    vi.stubGlobal("fetch", vi.fn(async () => new Response(new ReadableStream({ cancel }))));
    await expect(apiDownload("/audit-stalled-body", { timeoutMs: 20 })).rejects.toMatchObject({ kind: "timeout" });
    expect(cancel).toHaveBeenCalledTimes(1);
  });
  it("declared oversized responses cancel their body before refusal", async () => {
    const cancel = vi.fn();
    const bytes = 256 * 1024 * 1024 + 1;
    vi.stubGlobal("fetch", vi.fn(async () => new Response(new ReadableStream({ cancel }), {
      headers: { "content-length": String(bytes) },
    })));
    await expect(apiDownload("/audit-oversized-declared")).rejects.toMatchObject({ kind: "payload" });
    expect(cancel).toHaveBeenCalledTimes(1);
  });
  it("a public health response cannot hide a missing token", async () => {
    const f = vi.fn(async (input: RequestInfo | URL) => String(input).endsWith("/health") ? jsonResponse({ status: "ok" }) : jsonResponse({ error: { message: "token required" } }, 401));
    vi.stubGlobal("fetch", f);
    const result = await probeHealth("/api", null);
    expect(result.ok).toBe(false);
    expect(result.error?.kind).toBe("unauthorized");
    expect(f).toHaveBeenCalledTimes(2);
  });
  it("display redaction masks query credentials, cookies and prototype keys without mutation", () => {
    const value = JSON.parse('{"url":"https://user:pass@host/api?api_key=verysecret","cookie":"session-secret","__proto__":{"safe":1},"title":"safe"}') as Record<string, unknown>;
    const masked = deepMask(value);
    const text = JSON.stringify(masked);
    expect(text).not.toContain("verysecret");
    expect(text).not.toContain("session-secret");
    expect(text).not.toContain("user:pass");
    expect(Object.prototype.hasOwnProperty.call(masked, "__proto__")).toBe(true);
    expect((masked as Record<string, unknown>).title).toBe("safe");
    expect(JSON.stringify(value)).toContain("verysecret");
  });
  it("stops redaction traversal at its node budget without reading later values", () => {
    const wide: Record<string, unknown> = {};
    let reads = 0;
    for (let index = 0; index < 100_000; index += 1) {
      Object.defineProperty(wide, `field${index}`, {
        enumerable: true,
        configurable: true,
        get: () => { reads += 1; return `value-${index}`; },
      });
    }
    const masked = deepMask(wide, 5) as Record<string, unknown>;
    expect(reads).toBe(4); // root plus four values fit; the remaining 99,996 are never read
    expect(Object.keys(masked)).toContain("__truncated");
    expect(Object.keys(masked).length).toBeLessThanOrEqual(6);
    expect(JSON.stringify(masked)).not.toContain("value-99999");
  });
});
