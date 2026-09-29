import { describe, expect, it, vi } from "vitest";
import { apiGet } from "../../lib/api";
import { jsonResponse } from "./helpers";

describe("dedup + cache (§10)", () => {
  it("identical concurrent GETs share one request and one cache entry", async () => {
    const fetchMock = vi.fn(
      async () => {
        await new Promise((r) => setTimeout(r, 10));
        return jsonResponse({ v: 42 });
      },
    ) as unknown as typeof fetch;
    vi.stubGlobal("fetch", fetchMock);
    const [a, b] = await Promise.all([apiGet("/stats"), apiGet("/stats")]);
    expect(a).toEqual({ v: 42 });
    expect(b).toEqual(a);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const c = await apiGet("/stats"); // TTL cache hit
    expect(c).toEqual(a);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("noCache pollers bypass sharing but still dedupe per-flight", async () => {
    const fetchMock = vi.fn(async () => jsonResponse({ t: Date.now() })) as unknown as typeof fetch;
    vi.stubGlobal("fetch", fetchMock);
    await Promise.all([apiGet("/health", { noCache: true }), apiGet("/health", { noCache: true })]);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});
