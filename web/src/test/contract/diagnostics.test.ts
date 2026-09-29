/**
 * Contract test (GET /diagnostics).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import diagnostics from "../fixtures/diagnostics.json";

describe("GET /diagnostics contract", () => {
  it("components are {detail,name,status} with health vocabulary", () => {
    for (const c of diagnostics.components) {
      expect(Object.keys(c).sort()).toEqual(["detail", "name", "status"]);
      expect(["healthy", "degraded", "disabled", "error"]).toContain(c.status);
    }
    expect(["ok", "healthy", "degraded", "error", "disabled"]).toContain(diagnostics.overall);
  });
});
