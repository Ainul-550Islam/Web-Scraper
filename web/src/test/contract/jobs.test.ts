/**
 * Contract test (GET /jobs).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import jobs from "../fixtures/jobs.json";

describe("GET /jobs contract", () => {
  it("current job + note; per-target counters typed", () => {
    expect(Array.isArray(jobs.job.jobs)).toBe(true);
    expect(typeof jobs.note).toBe("string");
    for (const j of jobs.job.jobs)
      for (const k of ["blocked", "failed", "quarantined", "revalidated", "written"])
        expect(typeof (j as Record<string, unknown>)[k], `${j.target}.${k}`).toBe("number");
  });
});
