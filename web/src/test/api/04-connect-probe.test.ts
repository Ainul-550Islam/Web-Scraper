import { describe, expect, it, vi } from "vitest";
import { probeHealth } from "../../lib/api";
import { jsonResponse, neverEndingFetch } from "./helpers";

describe("connect probe (§3)", () => {
  it("probeHealth reports ok + latency, and normalizes failure kinds", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => jsonResponse({ status: "ok", running: true })) as unknown as typeof fetch);
    const good = await probeHealth("/api", null);
    expect(good.ok).toBe(true);
    expect(good.latencyMs).toBeTypeOf("number");

    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        jsonResponse({ error: { code: "unauthorized", message: "bearer token required for non-loopback bind" } }, 401),
      ) as unknown as typeof fetch,
    );
    const bad = await probeHealth("/api", "unit-test-secret");
    expect(bad.ok).toBe(false);
    expect(bad.error?.kind).toBe("unauthorized");
    expect(bad.error?.message).toContain("bearer token required");
  });

  it("probe never hangs on a dead port: it time-boxes to 4 s", async () => {
    vi.stubGlobal("fetch", neverEndingFetch());
    const r = await probeHealth("/api", null, 30);
    expect(r.ok).toBe(false);
    expect(r.error?.kind).toBe("timeout");
  });
});
