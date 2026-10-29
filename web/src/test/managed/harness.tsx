/**
 * Managed-surface test harness.
 *
 * `mountPage` in ../harness stubs fetch with a path→fixture map, which cannot
 * distinguish a GET from the POST that follows it. The managed pages do real
 * mutations, so this one routes on method and records every call.
 */
import type { ReactElement } from "react";
import { vi } from "vitest";
import { render, waitFor } from "@testing-library/react";
import { ThemeProvider } from "../../hooks/useTheme";
import { ToastProvider } from "../../hooks/useToasts";
import { ConnectionProvider } from "../../lib/connection";
import { Toasts } from "../../components/Overlays";
import { invalidateCache } from "../../lib/api";

export interface ManagedCall {
  method: string;
  path: string;
  search: string;
  body: string | undefined;
  headers: Record<string, string>;
}

export type RouteHandler = (call: ManagedCall) => unknown;

/** A handler result: a plain object is a 200 JSON body. */
export type RouteResult =
  | unknown
  | { __status: number; body: unknown }
  | Error;

export async function mountManaged(
  el: ReactElement,
  routes: Record<string, RouteHandler | RouteResult>,
): Promise<{
  calls: ManagedCall[];
  findByText: ReturnType<typeof render>["findByText"];
  getByText: ReturnType<typeof render>["getByText"];
  getByRole: ReturnType<typeof render>["getByRole"];
  queryByText: ReturnType<typeof render>["queryByText"];
  container: HTMLElement;
}> {
  const calls: ManagedCall[] = [];
  const stub = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(
      typeof input === "string" ? input : (input as Request).url,
      "http://localhost",
    );
    const path = url.pathname.replace(/^\/api(?=\/|$)/, "");
    const raw = init?.headers as Record<string, string> | Headers | undefined;
    const headers: Record<string, string> =
      !raw ? {} : raw instanceof Headers ? Object.fromEntries(raw.entries()) : { ...raw };
    const call: ManagedCall = {
      method: (init?.method ?? "GET").toUpperCase(),
      path,
      search: url.search,
      body: init?.body ? String(init.body) : undefined,
      headers,
    };
    calls.push(call);
    if (path === "/v1/admin/usage") console.log("USAGECALL", call.method, call.search, "route?", typeof routes[path], JSON.stringify(routes[path])?.slice(0, 80));
    const route = routes[path];
    if (!route) {
      return json({ error: { code: "not_found", message: `no route ${path}` } }, 404);
    }
    // A handler may be async; the response must wait for it, not stringify the promise.
    const out = typeof route === "function" ? await route(call) : route;
    if (out instanceof Error) throw out;
    return json(out, 200);
  });
  vi.stubGlobal("fetch", stub);
  invalidateCache();
  const utils = render(
    <ThemeProvider>
      <ToastProvider>
        <ConnectionProvider>{el}<Toasts /></ConnectionProvider>
      </ToastProvider>
    </ThemeProvider>,
  );
  return {
    calls,
    container: utils.container,
    findByText: utils.findByText.bind(utils),
    getByText: utils.getByText.bind(utils),
    getByRole: utils.getByRole.bind(utils),
    queryByText: utils.queryByText.bind(utils),
  };
}

function json(body: unknown, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** Wait until some recorded call matches. `after` skips calls recorded before
 *  that index, so a filter action is matched rather than the initial read. */
export async function untilCall(
  calls: ManagedCall[],
  pred: (call: ManagedCall) => boolean,
  what: string,
  opts: { after?: number } = {},
): Promise<ManagedCall> {
  let hit: ManagedCall | undefined;
  await waitFor(
    () => {
      const from = opts.after ?? 0;
      const found = calls.find((c, i) => i >= from && pred(c));
      if (!found) throw new Error(`waiting for ${what}`);
      hit = found;
    },
    { timeout: 3000 },
  );
  return hit!;
}

/** The fixtures the managed pages read. Shapes match the Rust handlers. */
export const tenantFixture = {
  schema: 1,
  tenant_count: 1,
  key_count: 1,
  tenants: [
    { id: "t-demo", name: "Demo", status: "active", created_at: 1_700_000_000 },
  ],
};

export const keyFixture = {
  schema: 1,
  keys: [
    {
      id: "key-1",
      tenant_id: "t-demo",
      key_hash: "9f2c…",
      prefix: "hv_live_ab12cd",
      scopes: ["scrape", "jobs_read", "usage_read"],
      limits: {
        rps: 5,
        burst: 10,
        max_concurrent_jobs: 2,
        monthly_quota: 100_000,
        max_response_bytes: 16 * 1024 * 1024,
        allowed_domains: ["example.com"],
        scopes: ["scrape", "jobs_read", "usage_read"],
      },
      status: "active",
      expires_at: null,
      created_at: 1_700_000_000,
      last_used_at: 1_700_000_100,
    },
  ],
};

export const usageFixture = {
  schema: 1,
  since_ms: 1_700_000_000_000,
  until_ms: 1_700_086_400_000,
  generated_at_ms: 1_700_086_400_000,
  key_count: 1,
  totals: {
    key_id: "",
    tenant_id: "",
    requests: 3,
    ok: 2,
    refused: 1,
    errors: 0,
    units: 24,
    bytes: 48_000,
    render_none: 2,
    render_browser: 1,
    screenshots: 0,
    first_ts_ms: 1_700_000_000_000,
    last_ts_ms: 1_700_086_399_000,
  },
  by_key: [
    {
      key_id: "key-1",
      tenant_id: "t-demo",
      requests: 3,
      ok: 2,
      refused: 1,
      errors: 0,
      units: 24,
      bytes: 48_000,
      render_none: 2,
      render_browser: 1,
      screenshots: 0,
      first_ts_ms: 1_700_000_000_000,
      last_ts_ms: 1_700_086_399_000,
    },
  ],
  by_day: [{ day: "2023-11-14", requests: 3, units: 24, bytes: 48_000 }],
  ledger_head: { seq: 7, hash: "abc123" },
};
