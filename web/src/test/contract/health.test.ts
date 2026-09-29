/**
 * Contract test (GET /health).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import health from "../fixtures/health.json";

describe("GET /health contract", () => {
  it("is exactly {running, status} — no version field is ever exposed", () => {
    expect(Object.keys(health).sort()).toEqual(["running", "status"]);
    expect(typeof health.running).toBe("boolean");
    expect(typeof health.status).toBe("string");
  });
});
