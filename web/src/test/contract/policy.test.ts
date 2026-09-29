/**
 * Contract test (GET /policy).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import policy from "../fixtures/policy.json";

describe("GET /policy contract (honesty clauses)", () => {
  it("states plainly that fingerprint spoofing is NOT implemented", () => {
    expect(policy.fingerprint_spoofing).toBe("not implemented, by design");
  });
  it("never frames proxies as block-evasion and honors refusals", () => {
    expect(policy.proxy_rotation).toContain("never used to evade blocks");
    expect(policy.refusals).toContain("Retry-After");
  });
});
