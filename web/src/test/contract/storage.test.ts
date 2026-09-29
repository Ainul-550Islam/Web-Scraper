/**
 * Contract test (GET /storage).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import storage from "../fixtures/storage.json";

describe("GET /storage contract", () => {
  it("every store reports configured/exists/bytes/kind/path", () => {
    for (const [name, st] of Object.entries(storage)) {
      for (const k of ["configured", "exists", "bytes", "kind", "path"])
        expect(k in st, `${name}.${k}`).toBe(true);
      expect(typeof (st as { configured: boolean }).configured).toBe("boolean");
      expect((st as { bytes: number | null }).bytes === null || typeof (st as { bytes: number | null }).bytes === "number").toBe(true);
    }
  });
});
