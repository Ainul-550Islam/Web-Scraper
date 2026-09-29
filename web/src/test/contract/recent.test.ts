/**
 * Contract test (GET /runs/recent).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import recent from "../fixtures/recent.json";

describe("GET /runs/recent contract", () => {
  it("is an array of run summaries with per-target counters", () => {
    expect(Array.isArray(recent)).toBe(true);
    for (const r of recent) {
      expect(typeof r.run).toBe("string");
      expect(typeof r.duration_ms).toBe("number");
      expect(typeof r.respect_robots).toBe("boolean");
      expect(r.summary.records).toBeGreaterThanOrEqual(0);
      for (const [target, c] of Object.entries(r.summary.per_target)) {
        expect(typeof target).toBe("string");
        for (const key of ["blocked", "failed", "quarantined", "revalidated", "written"])
          expect(typeof (c as Record<string, number>)[key], `${target}.${key}`).toBe("number");
      }
    }
  });
});
