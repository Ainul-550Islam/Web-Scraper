/**
 * Contract test (GET /events).
 * Pins the JSON envelope the console renders against the engine's captured
 * responses (web/src/test/fixtures, ADR 0004). Deliberately value-tolerant
 * on volatile data, strict on structure: field sets, types, vocabularies.
 */
import { describe, expect, it } from "vitest";
import events from "../fixtures/events_limit_4.json";

describe("GET /events contract (limit honored)", () => {
  it("ring snapshot keeps seq ordering monotone per newest_first flag", () => {
    expect(events.limit).toBeGreaterThanOrEqual(events.events.length);
    expect(typeof events.last_seq).toBe("number");
    const seqs = events.events.map((e: { seq: number }) => e.seq);
    const sorted = [...seqs].sort((a, b) => (events.newest_first ? b - a : a - b));
    expect(seqs).toEqual(sorted);
  });
  it("rows are {detail,job,kind,seq,ts} only", () => {
    for (const e of events.events)
      expect(Object.keys(e).sort()).toEqual(["detail", "job", "kind", "seq", "ts"]);
  });
});
