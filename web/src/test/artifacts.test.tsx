/**
 * Artifacts: the browser over what jobs produced — filtering, verification
 * and download, all server-side, over the recorded `control_jobs` fixture.
 *
 * A deliberate part of the contract: the page derives its list from the
 * retained job list because there is no artifact-index endpoint, and the tests
 * pin that honesty (no invented endpoint, no invented state).
 */
import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import ArtifactsPage from "../pages/Artifacts";
import { ArtifactList, isDeliveryRole } from "../components/patterns/ArtifactList";
import { mountPage, CONTROL_FIXTURE_IDS } from "./harness";
import { auth } from "../lib/api";

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

describe("artifact roles", () => {
  it("knows which roles take part in a manifest-verified delivery", () => {
    expect(isDeliveryRole("payload")).toBe(true);
    expect(isDeliveryRole("manifest")).toBe(true);
    expect(isDeliveryRole("checksums")).toBe(true);
    expect(isDeliveryRole("bundle")).toBe(true);
    // the run output is a real file, but it has no manifest to check against
    expect(isDeliveryRole("run_output")).toBe(false);
  });
});

describe("Artifacts page", () => {
  it("lists every artifact the retained jobs report", async () => {
    const { container } = await mountPage(<ArtifactsPage />);
    await waitFor(() => expect(container.querySelectorAll('[data-testid="artifact-row"]').length).toBe(7));
    const text = document.body.textContent ?? "";
    // job-1's export (6) + job-2's run output (1)
    expect(text).toContain("records.csv");
    expect(text).toContain("checksums.txt");
    expect(text).toContain("records.jsonl");
  });

  it("summarises bytes and readiness from the rows themselves", async () => {
    await mountPage(<ArtifactsPage />);
    await waitFor(() => expect(rowFor("records.csv")).toBeTruthy());
    await waitFor(() => expect(document.body.textContent).toContain("Bytes retained"));
    expect(document.body.textContent).toContain("Ready");
  });

  it("filters by role and by name", async () => {
    const { container } = await mountPage(<ArtifactsPage />);
    await waitFor(() => expect(container.querySelectorAll('[data-testid="artifact-row"]').length).toBe(7));

    const roleSelect = [...container.querySelectorAll("select")].find(
      (s) => s.getAttribute("aria-label") === "Role",
    ) as HTMLSelectElement;
    fireEvent.change(roleSelect, { target: { value: "manifest" } });
    await waitFor(() => expect(container.querySelectorAll('[data-testid="artifact-row"]').length).toBe(1));
    expect(rowFor("manifest.json")).toBeTruthy();

    fireEvent.change(roleSelect, { target: { value: "" } });
    await waitFor(() => expect(container.querySelectorAll('[data-testid="artifact-row"]').length).toBe(7));

    const search = [...container.querySelectorAll("input")].find(
      (i) => i.getAttribute("aria-label") === "Search",
    ) as HTMLInputElement;
    fireEvent.change(search, { target: { value: "zip" } });
    await waitFor(() => expect(container.querySelectorAll('[data-testid="artifact-row"]').length).toBe(1));
    expect(rowFor("records.zip")).toBeTruthy();
  });

  it("verifies through the server and shows the recorded verdict", async () => {
    const { calls } = await mountPage(<ArtifactsPage />);
    await waitFor(() => expect(rowFor("records.csv")).toBeTruthy());
    fireEvent.click(buttonIn(rowFor("records.csv"), "Verify"));
    await waitFor(() =>
      expect(calls.some((c) => c.path === `/artifacts/${CONTROL_FIXTURE_IDS.artifact}/verify`)).toBe(true),
    );
    await waitFor(() => expect(document.body.textContent).toContain("verification ok"));
    expect(document.body.textContent).toContain("4 record(s) across 4 file(s)");
  });

  it("downloads by registry id and never by path", async () => {
    // the token is a header, never a query parameter: prove it by setting one
    auth.set("artifacts-test-token-0123456789");
    const { calls } = await mountPage(<ArtifactsPage />);
    await waitFor(() => expect(rowFor("records.csv")).toBeTruthy());
    fireEvent.click(buttonIn(rowFor("records.csv"), "Download"));
    await waitFor(() =>
      expect(calls.some((c) => c.method === "GET" && c.path === `/artifacts/${CONTROL_FIXTURE_IDS.artifact}`)).toBe(true),
    );
    const call = calls.find((c) => c.path === `/artifacts/${CONTROL_FIXTURE_IDS.artifact}`)!;
    expect(call.path).not.toContain("..");
    expect(call.headers.authorization).toBe("Bearer artifacts-test-token-0123456789");
    // and it is not smuggled into the URL
    expect(call.search).toBe("");
    auth.set(null);
  });

  it("does not offer verification for the run output — it has no manifest", async () => {
    const { container } = await mountPage(<ArtifactsPage />);
    await waitFor(() => expect(container.querySelectorAll('[data-testid="artifact-row"]').length).toBe(7));
    const runOut = [...container.querySelectorAll('[data-testid="artifact-row"]')].find((r) =>
      (r.textContent ?? "").includes("run_output"),
    ) as HTMLElement;
    expect(runOut).toBeTruthy();
    const labels = [...runOut.querySelectorAll("button")].map((b) => b.textContent);
    expect(labels).toContain("Download");
    expect(labels).not.toContain("Verify");
  });

  it("shows the metadata route for one artifact when the address asks", async () => {
    window.location.hash = `#/artifacts?artifact=${CONTROL_FIXTURE_IDS.artifact}`;
    const { calls } = await mountPage(<ArtifactsPage />);
    await waitFor(() => expect(document.body.textContent).toContain(`Artifact ${CONTROL_FIXTURE_IDS.artifact}`));
    await waitFor(() => expect(document.body.textContent).toContain("verification ok"));
    expect(calls.some((c) => c.path === `/control/artifacts/${CONTROL_FIXTURE_IDS.artifact}`)).toBe(true);
    // metadata, not bytes: no path anywhere on the page
    expect(document.body.textContent).not.toContain("/tmp/");
    window.location.hash = "#/artifacts";
  });

  it("disables the download when the server marks an artifact unverified", async () => {
    const { container } = await mountPage(<ArtifactsPage />, {
      "/control/jobs": {
        __status: 200,
        body: {
          items: [
            {
              id: "job-7",
              operation: "export",
              state: "completed",
              resource: "csv",
              artifacts: [
                {
                  id: "art-9",
                  job: "job-7",
                  name: "records.csv",
                  role: "payload",
                  bytes: 120,
                  sha256: "deadbeef",
                  content_type: "text/csv; charset=utf-8",
                  state: "failed",
                  downloadable: false,
                  created_ts: 1_700_000_000,
                  verification: { state: "failed" },
                },
              ],
            },
          ],
          total: 1,
          in_flight: 0,
          caps: { max_concurrent_jobs: 8, max_queue_depth: 64 },
        },
      },
    });
    await waitFor(() => expect(container.querySelectorAll('[data-testid="artifact-row"]').length).toBe(1));
    expect(buttonIn(rowFor("records.csv"), "Download").disabled).toBe(true);
    // verification is still offered: that is how you find out why
    expect(buttonIn(rowFor("records.csv"), "Verify")).toBeTruthy();
  });

  it("renders nothing inventively when the job list is empty", async () => {
    const { container } = await mountPage(<ArtifactsPage />, {
      "/control/jobs": { __status: 200, body: { items: [], total: 0, in_flight: 0 } },
    });
    await waitFor(() => expect(document.body.textContent).toContain("No artifacts match"));
    expect(container.querySelectorAll('[data-testid="artifact-row"]')).toHaveLength(0);
  });
});

describe("ArtifactList", () => {
  it("renders one row per artifact with its hash and role", async () => {
    const { container } = await mountPage(
      <ArtifactList
        artifacts={[
          {
            id: "art-1",
            job: "job-1",
            name: "records.csv",
            role: "payload",
            bytes: 1024,
            sha256: "a".repeat(64),
            content_type: "text/csv; charset=utf-8",
            state: "ready",
            downloadable: true,
            created_ts: 1_700_000_000,
          },
        ]}
      />,
    );
    expect(container.querySelectorAll('[data-testid="artifact-row"]')).toHaveLength(1);
    expect(container.textContent).toContain("payload");
    expect(container.textContent).toContain("1.0 KiB");
  });

  it("says so when there is nothing to show", async () => {
    const { container } = await mountPage(<ArtifactList artifacts={[]} />);
    expect(container.textContent).toContain("No artifacts registered yet");
  });
});
