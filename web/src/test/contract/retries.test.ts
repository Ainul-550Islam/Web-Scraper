/**
 * Contract test (GET /retries).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import retries from "../fixtures/retries.json";

describe("GET /retries contract", () => {
  it("counters non-negative, delays an array, state known", () => {
    expect(retries.requests_retry).toBeGreaterThanOrEqual(0);
    expect(retries.accounting_retries).toBeGreaterThanOrEqual(0);
    expect(Array.isArray(retries.recent_delays_ms)).toBe(true);
    expect(["available", "unavailable"]).toContain(retries.state);
  });
});
