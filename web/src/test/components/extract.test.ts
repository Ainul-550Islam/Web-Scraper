import { describe, expect, it } from "vitest";
import { extract } from "../../components/Table/extract";
import type { Column } from "../../components/Table";

const col = (key: string): Column<Record<string, unknown>> => ({
  key,
  label: key,
  render: () => null,
});

describe("extract (Table/extract.tsx)", () => {
  it("stringifies numbers, blank for null/undefined, passes strings", () => {
    const row = { n: 42.5, s: "text", nul: null, missing: undefined };
    expect(extract(row, col("n"))).toBe("42.5");
    expect(extract(row, col("s"))).toBe("text");
    expect(extract(row, col("nul"))).toBe("");
    expect(extract(row, col("missing"))).toBe("");
    expect(extract(row, col("absent"))).toBe("");
  });
});
