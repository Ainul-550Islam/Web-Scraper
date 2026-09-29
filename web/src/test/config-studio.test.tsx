import { describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import ConfigStudio from "../features/config/ConfigStudio";
import { findControl, mountPage } from "./harness";

const editConcurrency = async (container: HTMLElement, v: string) => {
  const el = await findControl(container, "Concurrency");
  fireEvent.change(el, { target: { value: v } });
};

describe("ConfigStudio (draft-only editor over GET /config)", () => {
  it("seeds every control from the live document and reports a clean draft", async () => {
    const { findByText } = await mountPage(<ConfigStudio />);
    expect(await findByText(/draft matches server/)).toBeInTheDocument();
  });

  it("marks exactly one control dirty when edited and resets it back to the server value", async () => {
    const { container, findByText } = await mountPage(<ConfigStudio />);
    await findByText(/draft matches server/);
    const input = await findControl(container, "Concurrency");
    expect(input.value).toBe("3"); // from the real config.json capture
    await editConcurrency(container, "6");
    expect(await findByText(/1 changed in draft/)).toBeInTheDocument();
    expect(container.textContent).toContain("changed in draft"); // per-field note marker
    fireEvent.click(screen.getByRole("button", { name: "reset Concurrency to server value" }));
    await findByText(/draft matches server/);
    expect(input.value).toBe("3");
  });

  it("mirrors the engine's bounds locally: concurrency < 1 is rejected with an alert", async () => {
    const { container, findByText } = await mountPage(<ConfigStudio />);
    await findByText(/draft matches server/);
    await editConcurrency(container, "0");
    const alerts = await screen.findAllByRole("alert");
    expect(alerts.some((a) => /min|≥|1/.test(a.textContent ?? ""))).toBe(true);
  });

  it("exports the draft with every server-only key stripped", async () => {
    const { container, findByText } = await mountPage(<ConfigStudio />);
    await findByText(/draft matches server/);
    await editConcurrency(container, "4");
    let blob: Blob | null = null;
    vi.spyOn(URL, "createObjectURL").mockImplementationOnce((b) => {
      blob = b as Blob;
      return "blob:stub";
    });
    fireEvent.click(screen.getByRole("button", { name: /Export draft/ }));
    await waitFor(() => expect(blob).not.toBeNull());
    const text = await new Promise<string>((resolve, reject) => {
      const fr = new FileReader();
      fr.onload = () => resolve(String(fr.result));
      fr.onerror = () => reject(fr.error);
      fr.readAsText(blob as unknown as Blob);
    });
    const parsed = JSON.parse(text) as Record<string, unknown>;
    expect(parsed.concurrency).toBe(4); // edit landed in the file
    expect("_sources" in parsed).toBe(false);
    expect("_backends" in parsed).toBe(false);
    expect("config_sha256" in parsed).toBe(false);
    expect("validation" in parsed).toBe(false);
    expect(Array.isArray(parsed.targets)).toBe(true); // real payload preserved
  });

  it("never renders secret material — the token field is a disabled redaction sentinel", async () => {
    const { getAllByText } = await mountPage(<ConfigStudio />);
    const red = await screen.findAllByDisplayValue("<redacted>");
    expect(red.length).toBeGreaterThan(0);
    for (const el of red) expect((el as HTMLInputElement).disabled).toBe(true);
    expect(getAllByText("SECRET").length).toBe(red.length); // every secret control flagged, none rendered
  });
});
