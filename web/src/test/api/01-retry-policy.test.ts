import { describe, expect, it, vi } from "vitest";
import { ApiError, apiGet, apiPost } from "../../lib/api";
import { jsonResponse } from "./helpers";

describe("retry policy (§10)", () => {
  it("idempotent GET retries exactly once after a network failure", async () => {
    let n = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        n += 1;
        if (n === 1) throw new TypeError("Failed to fetch");
        return jsonResponse({ ok: 1 });
      }) as unknown as typeof fetch,
    );
    await expect(apiGet("/runtime", { noCache: true })).resolves.toEqual({ ok: 1 });
    expect(n).toBe(2);
  });

  it("503 is retried once for GET; persistent failure surfaces", async () => {
    let n = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        n += 1;
        return new Response("engine busy", { status: 503 });
      }) as unknown as typeof fetch,
    );
    await expect(apiGet("/runtime", { noCache: true })).rejects.toMatchObject({ kind: "server", status: 503 });
    expect(n).toBe(2);
  });

  it("POST is NEVER retried — outcome unknown means no auto-resend", async () => {
    let n = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        n += 1;
        if (n === 1) throw new TypeError("connection reset");
        return jsonResponse({ acked: 3 });
      }) as unknown as typeof fetch,
    );
    await expect(apiPost("/alerts/ack", { scope: "all" })).rejects.toBeInstanceOf(ApiError);
    expect(n).toBe(1);
  });

  it("429 is classified as slowdown and NOT retried by the client", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({ error: { code: "slow_down", message: "too fast" }, request_id: "req-9" }, 429),
    ) as unknown as typeof fetch;
    vi.stubGlobal("fetch", fetchMock);
    const e = await apiGet("/events", { noCache: true }).catch((x: unknown) => x as ApiError);
    expect(e).toBeInstanceOf(ApiError);
    expect((e as ApiError).kind).toBe("slowdown");
    expect((e as ApiError).requestId).toBe("req-9");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
