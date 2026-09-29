/**
 * Export Center: formats, validation, preview, prepare/verify/download and —
 * above all — no invented state. The page does not call an export endpoint
 * (the engine CLI owns generation), so these tests assert that it says so.
 */
import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import ExportCenter from "../pages/ExportCenter";
import ExportFormatPicker, { EXPORT_FORMATS, type ExportFormatId } from "../components/ExportFormatPicker";
import DataPreviewTable from "../components/DataPreviewTable";
import { mountPage } from "./harness";
import { REDACTION_MARK } from "../lib/redaction";

const HAS_SUBTLE = typeof crypto !== "undefined" && !!crypto.subtle?.digest;

function jsonlFile(rows: unknown[], name = "records.jsonl"): File {
  const text = rows.map((r) => JSON.stringify(r)).join("\n") + "\n";
  return new File([text], name, { type: "application/x-ndjson" });
}

const ROWS = [
  { run: "demo-run", target: "demo-catalog", url: "https://shop.test/p/0", fields: { title: "Mousetrap", price: 19.99, qty: 3, note: null } },
  { run: "demo-run", target: "demo-news", url: "https://shop.test/p/1", fields: { title: "Trap, deluxe", price: 24.5, qty: 0, note: "ok" } },
];

async function loadDataset(rows: unknown[] = ROWS) {
  const utils = await mountPage(<ExportCenter />);
  const input = utils.container.querySelector('input[aria-label="records file"]') as HTMLInputElement;
  fireEvent.change(input, { target: { files: [jsonlFile(rows)] } });
  // FileReader completion is async: wait for the dataset card to report a file.
  await waitFor(() => expect(utils.container.textContent).toContain("records.jsonl"));
  return utils;
}

describe("Export Center — source and validation", () => {
  afterEach(() => {
    URL.revokeObjectURL("blob:test");
  });

  it("shows an empty state until a dataset is loaded", async () => {
    const { findByText } = await mountPage(<ExportCenter />);
    expect(await findByText("No dataset loaded")).toBeTruthy();
    expect(await findByText(/Pick a JSONL or JSON-array export/)).toBeTruthy();
  });

  it("parses JSONL and reports the real column/record counts", async () => {
    const { container } = await loadDataset();
    await waitFor(() => expect(container.textContent).toContain("records.jsonl"));
    expect(container.textContent).toContain("4 column(s)");
  });

  it("rejects an unparsable file loudly instead of showing zeroes", async () => {
    const { container, findByText } = await mountPage(<ExportCenter />);
    const bad = new File(["{not json at all\n"], "broken.jsonl");
    fireEvent.change(container.querySelector('input[aria-label="records file"]') as HTMLInputElement, {
      target: { files: [bad] },
    });
    expect(await findByText(/no JSON records found/)).toBeTruthy();
  });
});

describe("Export Center — format picker", () => {
  it("renders every format with its extension and purpose", async () => {
    const { container } = await mountPage(
      <ExportFormatPicker available={EXPORT_FORMATS.map((f) => f.id)} selected={["csv"]} onToggle={() => {}} />,
    );
    for (const f of EXPORT_FORMATS) {
      expect(container.textContent).toContain(f.name);
      expect(container.textContent).toContain(f.ext);
      expect(container.textContent).toContain(f.purpose);
    }
  });

  it("marks a format the engine cannot write as unsupported and unselectable", async () => {
    const picked: ExportFormatId[] = [];
    const { container } = await mountPage(
      <ExportFormatPicker available={["csv", "jsonl"]} selected={[]} onToggle={(id) => picked.push(id)} />,
    );
    const parquet = container.querySelector('[data-format="parquet"]') as HTMLButtonElement;
    expect(parquet.getAttribute("data-supported")).toBe("no");
    expect(parquet.disabled).toBe(true);
    expect(container.textContent).toContain("not supported by this engine build");
    fireEvent.click(parquet);
    expect(picked).toEqual([]);
  });

  it("toggles selection state and reports it through aria-pressed", async () => {
    let selected: ExportFormatId[] = ["csv"];
    const { container } = await mountPage(
      <ExportFormatPicker
        available={EXPORT_FORMATS.map((f) => f.id)}
        selected={selected}
        onToggle={(id) => {
          selected = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id];
        }}
      />,
    );
    const xlsx = container.querySelector('[data-format="xlsx"]') as HTMLButtonElement;
    expect(xlsx.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(xlsx);
    expect(selected).toEqual(["csv", "xlsx"]);
  });
});

describe("Export Center — preview", () => {
  it("bounds rows and nulls render as a dash, not the word null", async () => {
    const many = Array.from({ length: 40 }, (_, i) => ({ id: i, note: i % 2 ? null : "x" }));
    const { container } = await mountPage(<DataPreviewTable columns={["id", "note"]} rows={many} maxRows={5} />);
    const table = container.querySelector('table[aria-label="data preview"]') as HTMLTableElement;
    expect(table.querySelectorAll("tbody tr").length).toBe(5);
    expect(container.textContent).toContain("5 of 40 row(s)");
    expect(table.textContent).toContain("—");
    expect(table.textContent).not.toContain("null");
  });

  it("masks credential-shaped fields on screen", async () => {
    const { container } = await mountPage(
      <DataPreviewTable columns={["api_key"]} rows={[{ api_key: "sk_live_abc123" }]} />,
    );
    const table = container.querySelector('table[aria-label="data preview"]') as HTMLTableElement;
    expect(table.textContent).toContain(REDACTION_MARK);
    expect(table.textContent).not.toContain("sk_live_abc123");
  });

  it("truncates long values and keeps the full text in the title", async () => {
    const long = "x".repeat(200);
    const { container } = await mountPage(<DataPreviewTable columns={["t"]} rows={[{ t: long }]} maxCellChars={10} />);
    const cell = container.querySelector(".dpt-cell") as HTMLTableCellElement;
    expect(cell.textContent).toBe("x".repeat(10) + "…");
    expect(cell.getAttribute("title")).toBe(long);
  });
});

describe("Export Center — prepare, verify, download", () => {
  it("composes the exact CLI command instead of claiming to generate files", async () => {
    const { container, getByLabelText } = await loadDataset();
    fireEvent.change(getByLabelText("dataset id"), { target: { value: "demo-run" } });
    const cmd = container.querySelector('code[aria-label="export command"]');
    expect(cmd?.textContent).toBe("harvest export --from records.jsonl --to ./delivery --format csv,jsonl --dataset demo-run");
  });

  it("prepares a real local JSONL with a real digest, then offers a download", async () => {
    if (!HAS_SUBTLE) return; // WebCrypto absent: the page disables the action
    const { container, getByLabelText, findByLabelText } = await loadDataset();
    fireEvent.change(getByLabelText("dataset id"), { target: { value: "demo-run" } });
    fireEvent.click(container.querySelector('[data-format="xlsx"]') as HTMLButtonElement);
    const prepareBtn = [...container.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("Prepare filtered JSONL"),
    ) as HTMLButtonElement;
    fireEvent.click(prepareBtn);
    const link = await findByLabelText(/download prepared dataset/i);
    expect(link.getAttribute("download")).toBe("dataset-demo-run.jsonl");
    await waitFor(() => expect(container.textContent).toMatch(/sha256 [0-9a-f]{12}/));
  });

  it("never shows a percentage-style progress indicator", async () => {
    const { container } = await loadDataset();
    // No animated bar, no invented percentage: the state is a named state.
    expect(container.querySelectorAll("progress").length).toBe(0);
    const metrics = container.querySelector(".grid-metrics") as HTMLElement;
    expect(metrics.textContent ?? "").not.toMatch(/\d{1,3}\s?%/);
    expect(metrics.textContent).toContain("State");
    expect(metrics.textContent).toMatch(/loaded|ready|reading file|idle/);
  });

  it("verifies a dropped manifest against its payload files", async () => {
    if (!HAS_SUBTLE) return;
    const { container } = await mountPage(<ExportCenter />);
    const body = "hello\n";
    // sha256 of "hello\n"
    const expected = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";
    const manifest = new File(
      [JSON.stringify({ records: 1, files: [{ name: "records.csv", sha256: expected, bytes: body.length }] })],
      "manifest.json",
    );
    fireEvent.change(container.querySelector('input[aria-label="manifest file"]') as HTMLInputElement, {
      target: { files: [manifest] },
    });
    fireEvent.change(container.querySelector('input[aria-label="payload files"]') as HTMLInputElement, {
      target: { files: [new File([body], "records.csv")] },
    });
    await waitFor(() => expect(container.textContent).toContain("1 promised file(s)"));
    const btn = [...container.querySelectorAll("button")].find((b) => b.textContent?.includes("Verify in browser"));
    fireEvent.click(btn as HTMLButtonElement);
    await waitFor(() => expect(container.textContent).toContain("1 of 1 file(s) match their digest"), { timeout: 4000 });
  });

  it("reports a tampered payload as mismatch, not as success", async () => {
    if (!HAS_SUBTLE) return;
    const { container } = await mountPage(<ExportCenter />);
    const expected = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03";
    const manifest = new File(
      [JSON.stringify({ records: 1, files: [{ name: "records.csv", sha256: expected, bytes: 6 }] })],
      "manifest.json",
    );
    fireEvent.change(container.querySelector('input[aria-label="manifest file"]') as HTMLInputElement, {
      target: { files: [manifest] },
    });
    fireEvent.change(container.querySelector('input[aria-label="payload files"]') as HTMLInputElement, {
      target: { files: [new File(["tampered\n"], "records.csv")] },
    });
    await waitFor(() => expect(container.textContent).toContain("1 promised file(s)"));
    const btn = [...container.querySelectorAll("button")].find((b) => b.textContent?.includes("Verify in browser"));
    fireEvent.click(btn as HTMLButtonElement);
    await waitFor(() => expect(container.textContent).toContain("mismatch"), { timeout: 4000 });
    expect(container.textContent).toContain("0 of 1 file(s) match their digest");
  });

  it("rejects a manifest without a files array", async () => {
    const { container, findByText } = await mountPage(<ExportCenter />);
    fireEvent.change(container.querySelector('input[aria-label="manifest file"]') as HTMLInputElement, {
      target: { files: [new File(['{"records":1}'], "manifest.json")] },
    });
    expect(await findByText(/no `files` array/)).toBeTruthy();
  });
});
