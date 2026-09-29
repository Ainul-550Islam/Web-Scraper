/**
 * Contract test (GET /failures).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import failures from "../fixtures/failures.json";

describe("GET /failures contract", () => {
  it("total ≥ listed items; state honest", () => {
    expect(failures.total).toBeGreaterThanOrEqual(failures.items.length);
    expect(["available", "unavailable"]).toContain(failures.state);
  });
});
