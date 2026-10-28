/**
 * Final pages: Run Detail, Config Diff, Delivery Verify, and the
 * redaction-first JsonInspector. Harness rules as everywhere: fixture-backed
 * fetches, no mocked page internals.
 */
import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import RunDetail from "../features/runs/RunDetail";
import ConfigDiff from "../features/config/ConfigDiff";
import Delivery from "../features/delivery/Delivery";
import { JsonInspector } from "../components/patterns/JsonInspector";
import { mountPage } from "./harness";

const HAS_SUBTLE = typeof crypto !== "undefined" && !!crypto.subtle?.digest;

describe("Run Detail", () => {
  afterEach(() => {
    window.location.hash = "";
  });

  it("asks for a run when no query parameter is present", async () => {
    const { findByText } = await mountPage(<RunDetail />);
    expect(await findByText("Select a run to inspect")).toBeInTheDocument();
  });

  it("resolves ?run= against the ledger and enriches from /recent", async () => {
    window.location.hash = "#/run?run=demo-run";
    const { container, findByText } = await mountPage(<RunDetail />);
    expect(await findByText("Run — demo-run")).toBeInTheDocument();
    // status label via statusLabel("ok") — the ledger says ok, the page says Completed
    expect((await screen.findAllByText("Completed")).length).toBeGreaterThan(0);
    expect(container.textContent).toContain("e9b681df"); // fmtHash(config_sha256)
    // /recent carries the full summary: per-target block renders
    expect(await findByText("demo-catalog")).toBeInTheDocument();
    expect(container.textContent).toContain("demo-news");
    // requests_total from the summary fixture is rendered in the ledger list
    expect((await screen.findAllByText("8")).length).toBeGreaterThan(0);
  });

  it("names an unknown run instead of rendering an empty lie", async () => {
    window.location.hash = "#/run?run=nope-run";
    const { findByText } = await mountPage(<RunDetail />);
    expect(await findByText("Run not found")).toBeInTheDocument();
  });
});

describe("Config Diff", () => {
  it("canonicalizes before diffing: key order alone is not a change", async () => {
    const { getByLabelText, findByText } = await mountPage(<ConfigDiff />);
    fireEvent.change(getByLabelText("left config document"), {
      target: { value: '{"b":1,"a":[2,{"z":9,"y":8}]}' },
    });
    fireEvent.change(getByLabelText("right config document"), {
      target: { value: '{"a":[2,{"y":8,"z":9}],"b":1}' },
    });
    expect(await findByText(/identical after canonicalization/)).toBeInTheDocument();
  });

  it("shows the changed lines and refuses invalid JSON loudly", async () => {
    const { container, getByLabelText, findByText, queryByText } = await mountPage(<ConfigDiff />);
    fireEvent.change(getByLabelText("left config document"), { target: { value: '{"max_retries":3}' } });
    fireEvent.change(getByLabelText("right config document"), { target: { value: '{"max_retries":5}' } });
    await findByText(/2 changed line\(s\)/);
    const pre = container.querySelector('pre[aria-label="line diff"]');
    expect(/-\s*"max_retries": 3/.test(pre?.textContent ?? "")).toBe(true);
    expect(/\+\s*"max_retries": 5/.test(pre?.textContent ?? "")).toBe(true);
    expect(queryByText(/Copy unified lines/i)).toBeInTheDocument(); // copy affordance for a real diff

    fireEvent.change(getByLabelText("left config document"), { target: { value: "{oops" } });
    expect(await findByText(/left: /)).toBeInTheDocument(); // parse error surfaced inline
  });
});

describe("JsonInspector redaction", () => {
  it("never renders or copies raw credential material", () => {
    const value = {
      authorization: { token: "hunter2", scheme: "Bearer" },
      proxies: [{ id: "p1", url: "http://user:s3cret@proxy.internal:8080" }],
      api_key: "sk-live-abcdef",
      webhook: "https://hooks.example/abc",
      safe: "visible",
    };
    const { container } = render(<JsonInspector value={value} openDepth={9} />);
    const text = container.textContent ?? "";
    for (const leak of ["hunter2", "s3cret", "sk-live-abcdef", "hooks.example/abc"]) {
      expect(text).not.toContain(leak);
    }
    expect(text).toContain("***"); // masked keys
    expect(text).toContain("visible"); // non-credential fields pass through
    expect(text).not.toContain("user:s3cret@"); // userinfo form also gone in any shape
    expect(container.textContent).toContain("Copy redacted JSON");
  });

  it("bounds depth-limit stubs in a very wide JSON tree", () => {
    const value: Record<string, unknown> = {};
    let branch = value;
    for (let depth = 0; depth < 63; depth += 1) {
      const next: Record<string, unknown> = {};
      branch[`level-${depth}`] = next;
      branch = next;
    }
    for (let index = 0; index < 10_000; index += 1) branch[`field-${index}`] = index;

    const { container } = render(
      <JsonInspector value={value} maxNodes={80} openDepth={64} redact={false} />,
    );
    const renderedNodes = container.querySelectorAll(".ji-row, .ji-toggle");
    expect(renderedNodes.length).toBeLessThanOrEqual(81); // one final omission stub may exceed the counted budget
    expect(container.textContent).toContain("node(s) beyond the 80-node render budget");
  });
});

describe("Delivery Verify", () => {
  it("states honestly what it cannot do outside a secure context", async () => {
    const { findByText } = await mountPage(<Delivery />);
    if (!HAS_SUBTLE) {
      expect(await findByText(/WebCrypto unavailable here/)).toBeInTheDocument();
    } else {
      expect(await findByText("No manifest loaded")).toBeInTheDocument();
    }
  });

  it.skipIf(!HAS_SUBTLE)("matches a real manifest against the same bytes", async () => {
    const text = '{"url":"http://x/1","fields":{}}\n';
    const payload = new File([text], "records.jsonl", { type: "application/x-ndjson" });
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
    const sha = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
    const manifest = new File(
      [
        JSON.stringify({
          identity: { records: 1, generated_ts: 1700000000 },
          files: [{ name: "records.jsonl", sha256: sha, bytes: payload.size, lines: 1 }],
        }),
      ],
      "manifest.json",
      { type: "application/json" },
    );

    const { container, getByLabelText, getByRole, findByText } = await mountPage(<Delivery />);
    fireEvent.change(getByLabelText("manifest file"), { target: { files: [manifest] } });
    await findByText(/promised file\(s\)/); // manifest parsed before the click
    fireEvent.change(getByLabelText("payload files"), { target: { files: [payload] } });
    fireEvent.click(getByRole("button", { name: "Verify in browser" }));
    await waitFor(() => expect(container.textContent).toContain("match their digest (100%)"));
    expect(container.querySelector("tbody .chip")?.textContent).toContain("match");
  });

  it.skipIf(!HAS_SUBTLE)("flips to mismatch when a byte differs", async () => {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode("aaa\n"));
    const sha = [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
    const manifest = new File(
      [JSON.stringify({ files: [{ name: "records.jsonl", sha256: sha, bytes: 4 }] })],
      "manifest.json",
    );
    const tampered = new File(["aab\n"], "records.jsonl", { type: "text/plain" });

    const { container, getByLabelText, getByRole, findByText } = await mountPage(<Delivery />);
    fireEvent.change(getByLabelText("manifest file"), { target: { files: [manifest] } });
    await findByText(/promised file\(s\)/);
    fireEvent.change(getByLabelText("payload files"), { target: { files: [tampered] } });
    fireEvent.click(getByRole("button", { name: "Verify in browser" }));
    await waitFor(() => expect(container.textContent).toContain("mismatch"));
    expect(await findByText(/not checked here/)).toBeInTheDocument(); // authority note stays visible
  });
});
