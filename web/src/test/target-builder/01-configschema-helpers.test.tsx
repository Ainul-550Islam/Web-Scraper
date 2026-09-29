import { describe, expect, it } from "vitest";

describe("configSchema helpers", () => {
  it("covers all 23 transform ops with exact arg tables", async () => {
    const { TRANSFORM_ARGS, TRANSFORM_OPS } = await import("../../lib/configSchema");
    expect(TRANSFORM_OPS).toHaveLength(23);
    for (const op of TRANSFORM_OPS) {
      expect(TRANSFORM_ARGS[op]).toBeInstanceOf(Array);
    }
    expect(TRANSFORM_ARGS.regex_replace.map((a) => a.name)).toEqual(["pattern", "replacement"]);
    expect(TRANSFORM_ARGS.lowercase).toHaveLength(0);
  });

  it("getPath/setPath round-trip nested config paths immutably", async () => {
    const { getPath, setPath } = await import("../../lib/configSchema");
    const doc = { default_policy: { requests_per_sec: 1.5 } };
    expect(getPath(doc, "default_policy.requests_per_sec")).toBe(1.5);
    const next = setPath(doc, "default_policy.requests_per_sec", 2.5);
    expect(getPath(next, "default_policy.requests_per_sec")).toBe(2.5);
    expect(getPath(doc, "default_policy.requests_per_sec")).toBe(1.5); // original untouched
  });

  it("enforces the honest-UA cross-field rule exactly like validate()", async () => {
    const { crossFieldErrors } = await import("../../lib/configSchema");
    const errs = crossFieldErrors({
      respect_robots: true,
      user_agent: "Mozilla/5.0 compatible",
      concurrency: 3,
      output: { kind: "jsonl" },
    } as never);
    expect(errs.some((e) => /mozilla/i.test(e))).toBe(true);
    expect(errs.some((e) => /path/.test(e))).toBe(true); // jsonl without path
    const ok = crossFieldErrors({
      respect_robots: true,
      user_agent: "harvest-bot/0.1",
      concurrency: 3,
      output: { kind: "jsonl", path: "out.jsonl" },
    } as never);
    expect(ok).toEqual([]);
  });

  it("validateControl mirrors the Rust bounds (ints, min/max, select options)", async () => {
    const { CONTROLS, validateControl } = await import("../../lib/configSchema");
    const conc = CONTROLS.find((c) => c.path === "concurrency")!;
    expect(validateControl(conc, "0")).toBeTruthy(); // min 1
    expect(validateControl(conc, "2.5")).toBeTruthy(); // int
    expect(validateControl(conc, 3)).toBeNull();
    const ua = CONTROLS.find((c) => c.path === "user_agent")!;
    expect(validateControl(ua, "harvest-bot/0.1")).toBeNull();
    const out = CONTROLS.find((c) => c.path === "output.kind")!;
    expect(validateControl(out, "kafka")).toBeTruthy(); // not in OUTPUT_KINDS
    expect(validateControl(out, "postgres")).toBeNull();
  });
});
