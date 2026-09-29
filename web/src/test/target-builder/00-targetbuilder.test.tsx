import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";
import type { TargetCfg } from "../../lib/types";
import { mountPage } from "../harness";
import { target0, mountBuilder, ctrlInput } from "./helpers";

describe("TargetBuilder", () => {
  it("renders the real seed URL and the read-only pipeline strip", async () => {
    const { getByDisplayValue, getByText } = await mountBuilder();
    expect(getByDisplayValue(target0.start_url)).toBeInTheDocument();
    expect(getByText("Target")).toBeInTheDocument();
    expect(getByText("Sink")).toBeInTheDocument();
  });

  it("propagates start_url edits into the draft via onChange", async () => {
    const { container, latest } = await mountBuilder();
    fireEvent.change(ctrlInput(container, "Start URL"), { target: { value: "http://example.com/new" } });
    expect(latest().start_url).toBe("http://example.com/new");
  });

  it("flips the conditional Sitemap URL field exactly with the sitemap toggle", async () => {
    // controlled harness: the builder is stateless, so edits only appear when
    // the (test-)parent re-renders it with the new draft — same as ConfigStudio
    const { TargetBuilder } = await import("../../features/config/TargetBuilder");
    const { useState } = await import("react");
    function Controlled() {
      const [t, setT] = useState<TargetCfg>(structuredClone(target0));
      return <TargetBuilder target={t} onChange={setT} onClose={() => undefined} />;
    }
    const { container, queryByText } = await mountPage(<Controlled />);
    const sw = container.querySelector('[aria-label="sitemap discovery"]') as HTMLButtonElement;
    const before = !!queryByText("Sitemap URL");
    fireEvent.click(sw);
    expect(!!queryByText("Sitemap URL")).toBe(!before);
  });

  it("gates Save on required transform args, then emits {op, pattern, replacement}", async () => {
    const { container, latest } = await mountBuilder();
    const add = [...container.querySelectorAll("button")].find((b) => b.textContent?.includes("+ add op"))!;
    fireEvent.click(add);
    const editor = container.querySelector('[style*="dashed"]') as HTMLElement;
    const sel = editor.querySelector("select")!;
    fireEvent.change(sel, { target: { value: "regex_replace" } });
    const save = [...editor.querySelectorAll("button")].find((b) => b.textContent?.startsWith("Add ")) as HTMLButtonElement;
    expect(save.disabled).toBe(true); // pattern+replacement missing
    const [pat, rep] = [...editor.querySelectorAll(".ctrl input")] as HTMLInputElement[];
    fireEvent.change(pat!, { target: { value: "a(b)c" } });
    expect(save.disabled).toBe(true);
    fireEvent.change(rep!, { target: { value: "x$1y" } });
    expect(save.disabled).toBe(false);
    fireEvent.click(save);
    const draft = latest();
    const list = draft.extract[Object.keys(draft.extract)[0]!]!;
    expect(list.transforms[list.transforms.length - 1]).toEqual({
      op: "regex_replace",
      pattern: "a(b)c",
      replacement: "x$1y",
    });
  });

  it("renames an extract field through the key input", async () => {
    const { container, latest } = await mountBuilder();
    const name = ctrlInput(container, "Field name");
    expect((name as HTMLInputElement).value).toBe(Object.keys(target0.extract)[0]);
    fireEvent.change(name, { target: { value: "title2" } });
    expect(latest().extract["title2"]).toBeDefined();
    expect(latest().extract[Object.keys(target0.extract)[0]!]).toBeUndefined();
  });
});
