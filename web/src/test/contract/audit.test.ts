/**
 * Contract test (GET /audit).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import audit from "../fixtures/audit_limit_4.json";

describe("GET /audit contract (empty when nothing recorded)", () => {
  it("is items-only, never a null body", () => {
    expect(Object.keys(audit)).toEqual(["items"]);
    expect(Array.isArray(audit.items)).toBe(true);
  });
});
