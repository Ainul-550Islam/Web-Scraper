/**
 * Contract test (GET /config).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import config from "../fixtures/config.json";

describe("GET /config contract (masked export)", () => {
  it("never carries a plaintext admin token", () => {
    expect("admin_token" in config).toBe(true);
    expect(config.admin_token).toBeNull();
  });
  it("ships backend booleans and a sha of the live document", () => {
    for (const b of ["postgres", "redis", "sqlite"])
      expect(typeof (config._backends as Record<string, boolean>)[b], b).toBe("boolean");
    expect(config.config_sha256).toMatch(/^[0-9a-f]{64}$/);
    expect(Array.isArray(config._warnings)).toBe(true);
    expect(Array.isArray(config.targets)).toBe(true);
  });
});
