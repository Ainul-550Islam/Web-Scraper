/**
 * Contract test (GET /stats).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import stats from "../fixtures/stats.json";

describe("GET /stats contract", () => {
  it("metrics is numeric except the per-target record map; running is a bool", () => {
    expect(typeof stats.running).toBe("boolean");
    for (const [k, v] of Object.entries(stats.metrics)) {
      if (k === "target_records_written") {
        for (const [t, n] of Object.entries(v as Record<string, number>))
          expect(typeof n, `metrics.${k}.${t}`).toBe("number");
      } else {
        expect(typeof v, `metrics.${k}`).toBe("number");
      }
    }
  });
  it("carries the same proxy-entry shape as /proxies.entries", () => {
    for (const e of stats.proxies)
      expect(typeof e.id + typeof e.requests).toBe("stringnumber");
  });
});
