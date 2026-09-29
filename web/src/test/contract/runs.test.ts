/**
 * Contract test (GET /runs).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import runs from "../fixtures/runs.json";

describe("GET /runs contract", () => {
  it("wraps rows in a {page,window,window_note} envelope", () => {
    expect(typeof runs.window).toBe("number");
    expect(runs.window_note).toContain("512");
    expect(Object.keys(runs.page).sort()).toEqual(["items", "limit", "offset", "total"]);
    expect(typeof runs.page.total).toBe("number");
  });
  it("every run row carries the summary columns and a sha-pinned config", () => {
    for (const row of runs.page.items) {
      expect(Object.keys(row).sort()).toEqual(
        ["config_sha256","duration_ms","error","index","quarantined","run","status","ts","written"],
      );
      expect(row.config_sha256).toMatch(/^[0-9a-f]{64}$/);
      expect(["ok", "partial", "failed", "cancelled"]).toContain(row.status);
      expect(typeof row.ts).toBe("number");
    }
  });
});
