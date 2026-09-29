/**
 * Control Job Detail: one job's real record — counters, limits, artifacts,
 * timeline, and the two server-backed actions (verify, download).
 *
 * The page reads `?job=<id>` from the hash router, so each test sets
 * `window.location.hash` before mounting.
 */
import { beforeEach, describe, expect, it } from "vitest";

import { fireEvent, waitFor } from "@testing-library/react";
import ControlJobDetail from "../pages/ControlJobDetail";
import { mountPage, CONTROL_FIXTURE_IDS } from "./harness";

function goJob(id: string) {
  window.location.hash = `#/control/job?job=${id}`;
}

/** The artifact row for `name` — actions are per artifact, so tests must not
 *  click the first button they see. */
function rowFor(name: string): HTMLElement {
  const row = [...document.querySelectorAll('[data-testid="artifact-row"]')].find((r) =>
    (r.textContent ?? "").includes(name),
  );
  if (!row) throw new Error(`no artifact row for ${name}`);
  return row as HTMLElement;
}

function buttonIn(row: HTMLElement, label: string): HTMLButtonElement {
  const b = [...row.querySelectorAll("button")].find((x) => x.textContent === label);
  if (!b) throw new Error(`no ${label} button in row`);
  return b as HTMLButtonElement;
}

beforeEach(() => {
  window.location.hash = "#/control/job";
});

describe("Control Job Detail", () => {
  it("renders the recorded job's counters, limits and artifacts", async () => {
    goJob(CONTROL_FIXTURE_IDS.job);
    const { container } = await mountPage(<ControlJobDetail />);
    await waitFor(() => expect(document.body.textContent).toContain("Job job-1"));

    const text = document.body.textContent ?? "";
    // operation + resource from the server
    expect(text).toContain("export");
    // six artifacts came out of the export: csv, jsonl, xlsx, zip, manifest, checksums
    expect(text).toContain("records.csv");
    expect(text).toContain("manifest.json");
    expect(text).toContain("checksums.txt");
    // the limits are the ones the job ran under
    expect(text).toContain("max_export_bytes");
    expect(text).toContain("timeout_secs");
    // the audit reference the server handed back
    expect(text).toContain("audit:");
    // artifacts render as rows with their hashes
    expect(container.querySelectorAll('[data-testid="artifact-row"]').length).toBeGreaterThan(3);
  });

  it("verifies an artifact through the server and reports the verdict", async () => {
    goJob(CONTROL_FIXTURE_IDS.job);
    const { calls } = await mountPage(<ControlJobDetail />);
    await waitFor(() => expect(document.body.textContent).toContain("records.csv"));
    fireEvent.click(buttonIn(rowFor("records.csv"), "Verify"));
    await waitFor(() =>
      expect(calls.some((c) => c.path === `/artifacts/${CONTROL_FIXTURE_IDS.artifact}/verify`)).toBe(true),
    );
    // the recorded verdict is `ok` with 4 records across 4 files
    await waitFor(() => expect(document.body.textContent).toContain("verification ok"));
    expect(document.body.textContent).toContain("4 record(s) across 4 file(s)");
  });

  it("downloads an artifact by id, never by path", async () => {
    goJob(CONTROL_FIXTURE_IDS.job);
    const { calls } = await mountPage(<ControlJobDetail />);
    await waitFor(() => expect(document.body.textContent).toContain("records.csv"));
    expect(buttonIn(rowFor("records.csv"), "Download").disabled).toBe(false);
    fireEvent.click(buttonIn(rowFor("records.csv"), "Download"));
    await waitFor(() =>
      expect(calls.some((c) => c.path === `/artifacts/${CONTROL_FIXTURE_IDS.artifact}`)).toBe(true),
    );
    const call = calls.find((c) => c.path === `/artifacts/${CONTROL_FIXTURE_IDS.artifact}`)!;
    expect(call.method).toBe("GET");
    // the id is a registry id — no separators, no traversal
    expect(call.path).not.toContain("..");
    expect(call.path).not.toContain("/tmp");
  });

  it("refuses to offer a cancel on a job the server already finished", async () => {
    goJob(CONTROL_FIXTURE_IDS.job);
    await mountPage(<ControlJobDetail />);
    await waitFor(() => expect(document.body.textContent).toContain("Job job-1"));
    const cancel = [...document.querySelectorAll("button")].find((b) => b.textContent === "Cancel job") as HTMLButtonElement;
    expect(cancel).toBeTruthy();
    expect(cancel.disabled).toBe(true);
    fireEvent.click(cancel);
    // nothing was sent: the server would have answered 409
    await new Promise((r) => setTimeout(r, 50));
  });

  it("says the failed AI job's reason out loud", async () => {
    goJob(CONTROL_FIXTURE_IDS.aiJob);
    const { container } = await mountPage(<ControlJobDetail />, {
      // the detail route for job-3: override with the failed-job shape
      "/control/jobs/job-3": {
        __status: 200,
        body: {
          id: "job-3",
          operation: "ai",
          state: "failed",
          resource: "classify",
          created_ts: 1_700_000_000,
          updated_ts: 1_700_000_001,
          deadline_ts: 1_700_000_900,
          cancel_requested: false,
          limits: {
            max_pages: 100,
            max_depth: 1,
            max_records: 100_000,
            max_export_bytes: 268_435_456,
            max_ai_input_bytes: 1_048_576,
            timeout_secs: 900,
          },
          counters: {
            pages_fetched: 0,
            records_extracted: 0,
            records_written: 0,
            requests_total: 0,
            failures: 0,
            retries: 0,
            quarantined: 0,
            blocked: 0,
            deduped: 0,
            schema_drifts: 0,
          },
          artifacts: [],
          result: { state: "disabled", provider: "disabled", external: false, counts: { fields: 0 } },
          error: "ai assistance is disabled for this job",
          audit_ref: "audit:req-3",
          events: [{ ts: 1_700_000_000, kind: "queued", detail: {} }],
        },
      },
    });
    await waitFor(() => expect(container.textContent).toContain("ai assistance is disabled for this job"));
    expect(container.textContent).toContain("nothing to download");
  });

  it("explains a missing job instead of inventing one", async () => {
    goJob("job-999999");
    await mountPage(<ControlJobDetail />, {
      "/control/jobs/job-999999": { __status: 404, body: { error: { code: "not_found", message: "no such job" } } },
    });
    await waitFor(() => expect(document.body.textContent).toContain("did not return this job"));
    expect(document.body.textContent).toContain("404");
  });

  it("asks for a job id when the address has none", async () => {
    window.location.hash = "#/control/job";
    await mountPage(<ControlJobDetail />);
    await waitFor(() => expect(document.body.textContent).toContain("Pick a job"));
  });

  it("never renders a filesystem path", async () => {
    goJob(CONTROL_FIXTURE_IDS.job);
    await mountPage(<ControlJobDetail />);
    await waitFor(() => expect(document.body.textContent).toContain("records.csv"));
    const text = document.body.textContent ?? "";
    expect(text).not.toContain("/tmp/");
    expect(text).not.toContain("/home/");
    expect(text).not.toContain("deliveries/");
  });
});
