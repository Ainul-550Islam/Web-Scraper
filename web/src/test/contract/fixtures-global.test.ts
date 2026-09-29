/**
 * Contract test (no version / integer ts).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import health from "../fixtures/health.json";
import config from "../fixtures/config.json";
import diagnostics from "../fixtures/diagnostics.json";
import events from "../fixtures/events_limit_4.json";
import proxies from "../fixtures/proxies.json";
import runtime from "../fixtures/runtime.json";
import stats from "../fixtures/stats.json";
import storage from "../fixtures/storage.json";

describe("fixture corpus-wide contract rules", () => {
  it("no endpoint ever exposes a version field", () => {
    for (const doc of [health, config, diagnostics, events, proxies, runtime, stats, storage])
      expect(Object.prototype.hasOwnProperty.call(doc, "version"), JSON.stringify(Object.keys(doc))).toBe(false);
  });
  it("every captured timestamp is a non-negative integer epoch", () => {
    for (const e of events.events) {
      expect(Number.isInteger(e.ts)).toBe(true);
      expect(e.ts).toBeGreaterThanOrEqual(0);
    }
    expect(Number.isInteger(diagnostics.generated_ts)).toBe(true);
  });
});
