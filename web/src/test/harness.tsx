/* eslint-disable react-refresh/only-export-components -- this module exports
 * fixtures and helpers (never a component); fast refresh is irrelevant to a
 * test harness, and splitting it would only hide the shared constants. */
/**
 * Test harness: mounts a page tree with the real providers and stubs fetch
 * with the captured live-API fixtures (src/test/fixtures/*.json, recorded
 * against a running harvest admin server). Pages therefore render against
 * byte-for-byte real responses, not invented shapes.
 */
import type { ReactElement } from "react";
import { vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { ThemeProvider } from "../hooks/useTheme";
import { ToastProvider } from "../hooks/useToasts";
import { ConnectionProvider } from "../lib/connection";
import { Toasts } from "../components/Overlays";
import { invalidateCache } from "../lib/api";

import runs from "./fixtures/runs.json";
import stats from "./fixtures/stats.json";
import alerts from "./fixtures/alerts.json";
import config from "./fixtures/config.json";
import recent from "./fixtures/recent.json";
import events from "./fixtures/events_limit_4.json";
import failures from "./fixtures/failures.json";
import retries from "./fixtures/retries.json";
import jobs from "./fixtures/jobs.json";
import workers from "./fixtures/workers.json";
import queue from "./fixtures/queue.json";
import storage from "./fixtures/storage.json";
import diagnostics from "./fixtures/diagnostics.json";
import pipeline from "./fixtures/pipeline.json";
import proxies from "./fixtures/proxies.json";
import audit from "./fixtures/audit_limit_4.json";
import health from "./fixtures/health.json";
import runtime from "./fixtures/runtime.json";
// control plane: recorded from a real `harvest serve` by
// `tools/record_control_fixtures.py` — not hand-written shapes
import controlJobs from "./fixtures/control_jobs.json";
import controlJob from "./fixtures/control_job.json";
import controlArtifact from "./fixtures/control_artifact.json";
import controlArtifactVerify from "./fixtures/control_artifact_verify.json";

export const FIXTURES: Record<string, unknown> = {
  "/runs": runs,
  "/stats": stats,
  "/alerts": alerts,
  "/config": config,
  "/recent": recent,
  "/events": events,
  "/failures": failures,
  "/retries": retries,
  "/jobs": jobs,
  "/workers": workers,
  "/queue": queue,
  "/storage": storage,
  "/diagnostics": diagnostics,
  "/pipeline": pipeline,
  "/proxies": proxies,
  "/audit": audit,
  "/health": health,
  "/runtime": runtime,
  "/live": runtime,
  // control plane — the ids are the ones the recording produced
  "/control/jobs": controlJobs,
  "/control/jobs/job-1": controlJob,
  "/control/artifacts/art-3": controlArtifact,
  "/artifacts/art-3/verify": controlArtifactVerify,
  // a download is bytes, not JSON; the harness serializes whatever is here,
  // and `apiDownload` only reads it back as an ArrayBuffer
  "/artifacts/art-3": "url,title,price\nhttps://catalogue.test/p/1,Item 1,$1.50\n",
};

/** The control-plane ids the recorded fixtures carry. */
export const CONTROL_FIXTURE_IDS = {
  job: "job-1",
  crawlJob: "job-2",
  aiJob: "job-3",
  artifact: "art-3",
} as const;

export interface Call {
  path: string;
  search: string;
  method: string;
  body?: string;
  headers: Record<string, string>;
}

/** Overrides replace/extend fixtures for one test (e.g. seeded alert rows). */
export async function mountPage(el: ReactElement, overrides: Record<string, unknown> = {}) {
  // precedence: explicit per-route override > __all (global fault) > fixture
  const all = overrides["__all"];
  const routes: Record<string, unknown> = Object.fromEntries(
    [...new Set([...Object.keys(FIXTURES), ...Object.keys(overrides)])].map((k) => [
      k,
      k === "__all"
        ? undefined
        : Object.prototype.hasOwnProperty.call(overrides, k)
          ? overrides[k]
          : all !== undefined
            ? all
            : FIXTURES[k],
    ]),
  );
  const calls: Call[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(
        typeof input === "string" ? input : (input as Request).url,
        "http://localhost",
      );
      const path = url.pathname.replace(/^\/api(?=\/|$)/, "");
      const raw = init?.headers as Record<string, string> | Headers | undefined;
      const headers: Record<string, string> =
        !raw ? {} : raw instanceof Headers ? Object.fromEntries(raw.entries()) : { ...raw };
      calls.push({
        path,
        search: url.search,
        method: (init?.method ?? "GET").toUpperCase(),
        body: init?.body ? String(init.body) : undefined,
        headers,
      });
      const hit = routes[path];
      if (hit instanceof Error) throw hit; // simulate network failure
      if (hit && typeof hit === "object" && "__status" in (hit as Record<string, unknown>)) {
        const spec = hit as { __status: number; body: unknown };
        return new Response(JSON.stringify(spec.body), {
          status: spec.__status,
          headers: { "content-type": "application/json" },
        });
      }
      if (hit instanceof Promise) {
        const resolved = await hit;
        return new Response(JSON.stringify(resolved), { status: 200, headers: { "content-type": "application/json" } });
      }
      if (hit === undefined) {
        return new Response(JSON.stringify({ error: { code: "not_found", message: `no route ${path}` } }), {
          status: 404,
          headers: { "content-type": "application/json" },
        });
      }
      return new Response(JSON.stringify(hit), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }),
  );
  invalidateCache();
  const utils = render(
    <ThemeProvider>
      <ToastProvider>
        <ConnectionProvider>{el}<Toasts /></ConnectionProvider>
      </ToastProvider>
    </ThemeProvider>,
  );
  return { ...utils, calls };
}

/** wait until predicate over recorded fetch calls holds (pollers are interval-driven) */
export async function until(
  pred: () => boolean,
  { timeout = 2500, step = 25 } = {},
): Promise<void> {
  const end = Date.now() + timeout;
  while (!pred()) {
    if (Date.now() > end) throw new Error("until(): timeout waiting for fetch call pattern");
    await new Promise((r) => setTimeout(r, step));
  }
}

/** find a control's <input>/<select> by the Field label text (throws sync) */
export function controlInput(container: HTMLElement, label: string): HTMLInputElement {
  const ctrl = [...container.querySelectorAll(".ctrl")].find((el) =>
    el.querySelector(".field-label")?.textContent?.trim().startsWith(label),
  );
  const input = ctrl?.querySelector("input,select,textarea") as HTMLInputElement | null;
  if (!input) throw new Error(`no control found for label "${label}"`);
  return input;
}

/** async version: waits for the studio's controls to mount after data lands */
export async function findControl(container: HTMLElement, label: string): Promise<HTMLInputElement> {
  let el!: HTMLInputElement;
  await waitFor(() => {
    el = controlInput(container, label);
  });
  return el;
}

export { runs, stats, alerts, config, events, queue, workers, storage, pipeline };
