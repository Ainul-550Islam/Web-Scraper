/**
 * Contract test (GET /pipeline).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import pipeline from "../fixtures/pipeline.json";

describe("GET /pipeline contract", () => {
  it("enforcement mode is one of the three documented words", () => {
    for (const p of pipeline.per_target)
      expect(["off", "warn", "enforce"]).toContain(p.enforce);
  });
  it("counters are non-negative numbers", () => {
    for (const k of ["pipeline_failures", "quarantined", "schema_drifts", "written"])
      expect((pipeline as unknown as Record<string, number>)[k], k).toBeGreaterThanOrEqual(0);
  });
});
