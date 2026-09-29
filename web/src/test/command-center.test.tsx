/**
 * Command Center + Event Stream — behavior on real recorded fixtures
 * (same harness every page test uses: fetch stubbed with byte-for-byte
 * `src/test/fixtures/*.json`, overrides for scenario switching).
 */
import { describe, expect, it } from "vitest";
import { fireEvent, screen, within } from "@testing-library/react";
import CommandCenter from "../features/command/CommandCenter";
import EventsPage from "../features/events/EventsPage";
import { mountPage, until } from "./harness";

describe("Command Center", () => {
  it("renders the live snapshot, lanes, and event tail from fixtures", async () => {
    const { getByText, findByText } = await mountPage(<CommandCenter />);
    expect(await findByText(/run “demo-run”/)).toBeInTheDocument(); // /runtime engine.run
    getByText("Queue");
    getByText("Storage lanes");
    getByText("Recent activity");
    getByText("…/out/demo-runs.jsonl"); // storage.history path, verbatim    // event tail renders through the shared timeline (kinds from /events fixture)
    expect((await screen.findAllByText("job_started")).length).toBeGreaterThan(0);
    expect(await screen.findByText(/job_completed/)).toBeInTheDocument();
  });

  it("shows honest idle between runs instead of faked zeroes", async () => {
    const { findByText, getByText } = await mountPage(<CommandCenter />, {
      "/runtime": { error: "no run active" },
    });
    expect(await findByText("idle — no run in progress")).toBeInTheDocument();
    getByText("Written"); // ledger fallback card, unit-labeled
    getByText("runs in ledger");
  });

  it("flags configured lanes that are missing on disk", async () => {
    const { findByText } = await mountPage(<CommandCenter />, {
      "/storage": {
        history: { kind: "history jsonl", path: "/srv/hist.jsonl", bytes: null, configured: true, exists: false },
      },
    });
    expect(await findByText(/configured lanes missing on disk: history/)).toBeInTheDocument();
  });

  it("keeps polling honest when the API is down", async () => {
    const { findByText } = await mountPage(<CommandCenter />, {
      __all: new Error("network down"),
    });
    expect(await findByText("runtime snapshot unreachable")).toBeInTheDocument();
  });
});

describe("Event Stream", () => {
  it("lists the fetched tail and opens details on click", async () => {
    const { findByText } = await mountPage(<EventsPage />);
    expect(await findByText(/ring 14 · last seq 14/)).toBeInTheDocument();
    const row = await findByText("#14 job_started");
    fireEvent.click(row);
    // selected event body renders through the redaction-first inspector
    const inspector = await screen.findByText("JSON");
    expect(within(inspector.closest("section")!).getByText(/Copy redacted JSON/)).toBeInTheDocument();
    expect(await screen.findByText(/redaction-masked/)).toBeInTheDocument();
  });

  it("passes kind + since_seq to the server as real query params, substring filter stays local", async () => {
    const { calls, findByText } = await mountPage(<EventsPage />);
    await findByText(/ring \d+/);
    const kindSelect = screen.getByLabelText("event kind");
    fireEvent.change(kindSelect, { target: { value: "job_failed" } });
    await until(() => calls.some((c) => c.path === "/events" && c.search.includes("kind=job_failed")));
    const sinceInput = screen.getByLabelText("since sequence");
    fireEvent.change(sinceInput, { target: { value: "10" } });
    await until(() => calls.some((c) => c.search.includes("since_seq=10")));
    // the "contains" field never triggers a request with a q param — it is
    // labeled and implemented as window-local narrowing only
    fireEvent.change(screen.getByLabelText(/substring filter/), { target: { value: "product/9" } });
    expect(await screen.findByText(/No fetched event matches the filter/)).toBeInTheDocument();
    expect(calls.some((c) => c.search.includes("q=product"))).toBe(false);
  });

  it("surfaces ring drops from the server, not invented completeness", async () => {
    const { findByText } = await mountPage(<EventsPage />, {
      "/events": {
        limit: 50,
        newest_first: true,
        ring_len: 50,
        dropped: 7,
        last_seq: 120,
        events: [],
      },
    });
    expect(await findByText(/ring dropped 7/)).toBeInTheDocument();
  });
});
