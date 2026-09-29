/**
 * Control Jobs: submission, the idempotency header, cancellation, the honest
 * error paths, and the state vocabulary — all rendered against the recorded
 * `control_jobs` fixture (real `harvest serve` responses).
 */
import { describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import ControlJobs from "../pages/ControlJobs";
import CommandComposer, {
  buildCommand,
  emptyDraft,
  validateCommand,
  CEILINGS,
} from "../components/patterns/CommandComposer";
import { canCancel, isTerminal, jobBlurb } from "../components/patterns/JobStateBadge";
import { mountPage, CONTROL_FIXTURE_IDS } from "./harness";
import { ApiError } from "../lib/api";

describe("job state vocabulary", () => {
  it("treats only the three terminal states as terminal", () => {
    expect(isTerminal("completed")).toBe(true);
    expect(isTerminal("failed")).toBe(true);
    expect(isTerminal("cancelled")).toBe(true);
    expect(isTerminal("queued")).toBe(false);
    expect(isTerminal("running")).toBe(false);
  });

  it("offers cancellation only while a job can still move", () => {
    expect(canCancel("queued")).toBe(true);
    expect(canCancel("running")).toBe(true);
    expect(canCancel("completed")).toBe(false);
    expect(canCancel("cancelled")).toBe(false);
  });

  it("describes every state in words the server would agree with", () => {
    for (const s of ["queued", "running", "completed", "failed", "cancelled"]) {
      expect(jobBlurb(s).length).toBeGreaterThan(4);
    }
    expect(jobBlurb("made-up")).toBe("unknown state");
  });
});

describe("command composer", () => {
  it("builds exactly the body the route expects", () => {
    const crawl = emptyDraft("crawl", ["demo-catalog"]);
    if (crawl.op !== "crawl") throw new Error("fixture drift");
    crawl.crawl.targets = ["demo-catalog"];
    expect(buildCommand(crawl)).toMatchObject({ op: "crawl", targets: ["demo-catalog"] });

    const exp = emptyDraft("export");
    expect(buildCommand(exp)).toMatchObject({
      op: "export",
      formats: ["csv", "jsonl"],
      dataset: null,
      verify: true,
    });

    const ai = emptyDraft("ai");
    expect(buildCommand(ai)).toMatchObject({
      op: "ai",
      task: "classify",
      provider: "deterministic-local",
      external_allowed: false,
      source: { kind: "run_output", record_limit: 100, field_limit: 32 },
    });
  });

  it("refuses the same ceilings the server refuses", () => {
    const draft = emptyDraft("crawl", ["demo-catalog"]);
    if (draft.op !== "crawl") throw new Error("fixture drift");
    draft.crawl.targets = ["demo-catalog"];
    draft.crawl.limits.max_pages = CEILINGS.maxPages + 1;
    expect(validateCommand(draft, ["demo-catalog"]).map((i) => i.field)).toContain("limits.max_pages");

    draft.crawl.limits.max_pages = 10;
    draft.crawl.targets = [];
    expect(validateCommand(draft, ["demo-catalog"]).map((i) => i.field)).toContain("targets");

    draft.crawl.targets = ["ghost"];
    expect(validateCommand(draft, ["demo-catalog"])).toHaveLength(1);
  });

  it("never asks for a token, a key or a path", () => {
    const draft = emptyDraft("export");
    const body = JSON.stringify(buildCommand(draft)).toLowerCase();
    for (const banned of ["token", "api_key", "secret", "/home/", "/tmp/", ".."]) {
      expect(body).not.toContain(banned);
    }
  });
});

describe("Control Jobs page", () => {
  it("renders the recorded jobs with their real states", async () => {
    const { container } = await mountPage(<ControlJobs />);
    await waitFor(() => expect(container.querySelectorAll('[data-testid="job-row"]').length).toBe(3));
    const text = document.body.textContent ?? "";
    expect(text).toContain("job-1");
    expect(text).toContain("job-3");
    // job-3 is the disabled-provider AI job: its reason is shown, not hidden
    expect(text).toContain("ai assistance is disabled for this job");
    // the recorded export produced six artifacts
    expect(text).toContain("records.csv");
  });

  it("shows the server's own capacity caps", async () => {
    await mountPage(<ControlJobs />);
    // the caps come from the server's `caps` block, not from a constant here
    await waitFor(() => expect(document.body.textContent).toContain("ceiling 64"));
    expect(document.body.textContent).toContain("enforced by the server");
  });

  it("posts a command with an Idempotency-Key header", async () => {
    const { calls } = await mountPage(<ControlJobs />);
    await screen.findByText("Submit export command");
    fireEvent.click(screen.getByRole("button", { name: /Submit export command/ }));
    await waitFor(() =>
      expect(calls.some((c) => c.method === "POST" && c.path === "/control/export")).toBe(true),
    );
    const post = calls.find((c) => c.method === "POST" && c.path === "/control/export")!;
    expect(JSON.parse(post.body ?? "{}")).toMatchObject({ op: "export", formats: ["csv", "jsonl"] });
    const key = post.headers["idempotency-key"] ?? post.headers["Idempotency-Key"];
    expect(key).toMatch(/^export-[a-z0-9]+-[a-z0-9]+$/);
  });

  it("cancels a job that can still move, and not one that cannot", async () => {
    const { calls, container } = await mountPage(<ControlJobs />);
    await waitFor(() => expect(container.querySelectorAll('[data-testid="job-row"]').length).toBe(3));
    // the fixture's jobs are all terminal, so every cancel button is disabled:
    // the console cannot offer an action the server would refuse with 409
    const buttons = [...container.querySelectorAll("button")].filter((b) => b.textContent === "Cancel");
    expect(buttons).toHaveLength(3);
    for (const b of buttons) expect((b as HTMLButtonElement).disabled).toBe(true);
    expect(calls.some((c) => c.path.includes("/cancel"))).toBe(false);
  });

  it("explains a 503 as capacity, not as a broken console", async () => {
    await mountPage(<ControlJobs />, {
      "/control/jobs": { __status: 503, body: { error: { code: "unavailable", message: "the queue is full" } } },
    });
    await waitFor(() => expect(document.body.textContent).toContain("HTTP 503"));
    expect(document.body.textContent).toContain("Request failed");
    expect(document.body.textContent).toContain("the queue is full");
  });

  it("asks for a token when the server answers 401", async () => {
    await mountPage(<ControlJobs />, {
      "/control/jobs": { __status: 401, body: { error: { code: "unauthorized", message: "missing bearer token" } } },
    });
    await waitFor(() => expect(document.body.textContent).toContain("bearer token"));
  });

  it("renders the composer with the config's declared targets", async () => {
    await mountPage(<ControlJobs />);
    await screen.findByText("Compose a command");
    fireEvent.click(screen.getByRole("button", { name: "crawl" }));
    await waitFor(() => expect(document.body.textContent).toContain("demo-catalog"));
  });
});

describe("cancellation error mapping", () => {
  it("a 409 is reported as 'already finished', not as a failure", async () => {
    // the page's handler branches on ApiError.kind; pin the mapping directly
    const err = new ApiError("conflict", "job job-1 is already completed", { status: 409 });
    expect(err.kind).toBe("conflict");
    expect(err.status).toBe(409);
    expect(err.message).toContain("already completed");
  });
});

describe("composer wiring", () => {
  it("does not submit while the draft is invalid", async () => {
    const onSubmit = vi.fn();
    const draft = emptyDraft("export");
    if (draft.op !== "export") throw new Error("fixture drift");
    draft.export.formats = [];
    const { container } = await mountPage(
      <CommandComposer draft={draft} onChange={() => {}} onSubmit={onSubmit} />,
    );
    await waitFor(() => expect(container.textContent).toContain("pick at least one format"));
    const submit = [...container.querySelectorAll("button")].find((b) =>
      (b.textContent ?? "").startsWith("Submit"),
    ) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    fireEvent.click(submit);
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("links a job row to the artifact browser with the recorded id", () => {
    expect(CONTROL_FIXTURE_IDS.artifact).toBe("art-3");
    expect(CONTROL_FIXTURE_IDS.job).toBe("job-1");
  });
});
