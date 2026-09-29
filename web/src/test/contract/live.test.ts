/**
 * Contract test (GET /stats/live).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import live from "../fixtures/live.json";

describe("GET /stats/live contract", () => {
  it("is a single boolean — never a richer object", () => {
    expect(Object.keys(live)).toEqual(["live"]);
    expect(typeof live.live).toBe("boolean");
  });
});
