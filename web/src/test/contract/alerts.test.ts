/**
 * Contract test (GET /alerts).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import alerts from "../fixtures/alerts.json";

describe("GET /alerts contract", () => {
  it("counters and a fired/notified pair always accompany items", () => {
    expect(Array.isArray(alerts.items)).toBe(true);
    for (const k of ["cooldown_secs", "fired", "notified"])
      expect(typeof (alerts as Record<string, unknown>)[k], k).toBe("number");
    expect(typeof alerts.min_severity).toBe("string");
  });
});
