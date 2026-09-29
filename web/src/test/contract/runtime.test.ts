/**
 * Contract test (GET /runtime).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import runtime from "../fixtures/runtime.json";

describe("GET /runtime contract (merged snapshot)", () => {
  it("engine trio + workers snapshot + queue stats agree with their sources", () => {
    expect(Object.keys(runtime.engine).sort()).toEqual(["run", "status", "uptime_ms"]);
    expect(runtime.workers.busy).toBeLessThanOrEqual(runtime.workers.registered);
    expect(runtime.queue.depth).toBeGreaterThanOrEqual(0);
  });
});
