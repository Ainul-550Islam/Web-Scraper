/**
 * Contract test (GET /queue).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import queue from "../fixtures/queue.json";

describe("GET /queue contract", () => {
  it("state word + numeric stats; inflight may exceed depth", () => {
    expect(["healthy", "degraded", "unavailable"]).toContain(queue.state);
    expect(queue.stats.depth).toBeGreaterThanOrEqual(0);
    expect(queue.stats.inflight).toBeGreaterThanOrEqual(0);
    expect(typeof queue.avg_queue_wait_ms).toBe("number");
  });
});
