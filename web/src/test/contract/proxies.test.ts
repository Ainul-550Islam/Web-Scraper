/**
 * Contract test (GET /proxies).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import proxies from "../fixtures/proxies.json";

describe("GET /proxies contract", () => {
  it("state buckets add up to total", () => {
    expect(proxies.healthy + proxies.degraded + proxies.down + proxies.unprobed).toBe(
      proxies.total,
    );
    expect(proxies.entries.length).toBe(proxies.total);
  });
  it("entries carry EWMA floats and a known state word", () => {
    for (const e of proxies.entries) {
      expect(["healthy", "degraded", "down", "unprobed"]).toContain(e.state);
      expect(typeof e.success_ewma).toBe("number");
      expect(typeof e.latency_ewma_ms).toBe("number");
    }
  });
});
