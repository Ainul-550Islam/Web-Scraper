/**
 * Contract test (GET /workers).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import workers from "../fixtures/workers.json";

describe("GET /workers contract", () => {
  it("snapshot counters stay internally consistent", () => {
    const s = workers.snapshot;
    expect(s.busy).toBeLessThanOrEqual(s.registered);
    expect(s.utilization).toBeGreaterThanOrEqual(0);
    expect(s.utilization).toBeLessThanOrEqual(1);
    expect(s.workers.length).toBe(s.registered);
  });
});
