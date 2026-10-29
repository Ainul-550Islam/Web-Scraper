/**
 * ApiKeys page (features/managed/keys/ApiKeys.tsx).
 *
 * Every assertion here is about what goes over the wire: the create body, the
 * scope validation that must happen before it, and the three key-id routes.
 */
import { describe, expect, it } from "vitest";
import { fireEvent, screen } from "@testing-library/react";
import { ApiKeys } from "../../features/managed/keys/ApiKeys";
import { controlInput } from "../harness";
import { mountManaged } from "./harness";
import {
  keyFixture,
  tenantFixture,
  untilCall,
  type ManagedCall,
  type RouteHandler,
} from "./harness";

const issued = {
  schema: 1,
  api_key: "hv_live_newsecretvalue",
  key: { ...keyFixture.keys[0]!, id: "key-2", prefix: "hv_live_zz99yy" },
};

/** The tenant list is polled; every other read is one-shot. */
function routes(overrides: Record<string, RouteHandler | unknown> = {}) {
  return {
    "/v1/admin/tenants": tenantFixture,
    "/v1/admin/keys": keyFixture,
    ...overrides,
  };
}

/** `runAction` POSTs to `/v1/admin/keys/{id}/{action}`; register the three. */
function keyActionRoutes(
  out: unknown,
  extra: Record<string, RouteHandler> = {},
): Record<string, RouteHandler | unknown> {
  const handler: RouteHandler = () => out;
  return {
    "/v1/admin/keys/key-1/rotate": handler,
    "/v1/admin/keys/key-1/suspend": handler,
    "/v1/admin/keys/key-1/revoke": handler,
    ...extra,
  };
}

async function mount(routes_: Record<string, RouteHandler | unknown>) {
  return mountManaged(<ApiKeys />, routes_);
}

describe("ApiKeys page", () => {
  it("renders the tenants and the tenant's keys from the server", async () => {
    const { findByText } = await mount(routes());
    expect(await findByText("Demo (t-demo)")).toBeInTheDocument();
    expect(await findByText("hv_live_ab12cd")).toBeInTheDocument();
    expect(screen.getByText("key-1")).toBeInTheDocument();
    // The scope name appears twice — once in the issue form, once per key row.
    expect(screen.getAllByText("Scrape")).toHaveLength(2);
    expect(screen.getByText("5 rps · burst 10")).toBeInTheDocument();
    expect(screen.getByText("domains: example.com")).toBeInTheDocument();
  });

  it("shows an empty state when the tenant has no keys", async () => {
    const { findByText } = await mount(
      routes({ "/v1/admin/keys": { schema: 1, keys: [] } }),
    );
    expect(await findByText("No keys yet")).toBeInTheDocument();
    expect(
      await findByText(/Issue one from the form on the left/),
    ).toBeInTheDocument();
  });

  it("creates a key with the scopes and limits the form collected", async () => {
    const { findByText, calls, container } = await mount(
      routes({ "/v1/admin/keys": (call: ManagedCall) => (call.method === "POST" ? issued : keyFixture) }),
    );
    await findByText("hv_live_ab12cd");

    // Widen the default scope set so the assertion is not just the default.
    fireEvent.click(screen.getByRole("checkbox", { name: "Write jobs" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Read usage" }));

    fireEvent.change(controlInput(container, "Requests / second"), {
      target: { value: "12.5" },
    });

    fireEvent.click(screen.getByRole("button", { name: "Create key" }));

    const post = await untilCall(
      calls,
      (c) => c.method === "POST" && c.path === "/v1/admin/keys",
      "the create POST",
    );
    const body = JSON.parse(post.body ?? "{}");
    expect(body).toEqual({
      tenant_id: "t-demo",
      scopes: ["scrape", "jobs_read", "jobs_write"],
      limits: {
        rps: 12.5,
        burst: 10,
        max_concurrent_jobs: 2,
        monthly_quota: 100_000,
        max_response_bytes: 16 * 1024 * 1024,
        allowed_domains: [],
      },
    });

    // The plaintext is revealed once, in a dialog.
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent(/hv_live_newsecretvalue/);
  });

  it("refuses to create a key with no scopes and does not post", async () => {
    const { findByText, calls } = await mount(
      routes({ "/v1/admin/keys": (call: ManagedCall) => (call.method === "POST" ? issued : keyFixture) }),
    );
    await findByText("hv_live_ab12cd");
    fireEvent.click(screen.getByRole("checkbox", { name: "Scrape" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Read jobs" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Read usage" }));
    fireEvent.click(screen.getByRole("button", { name: "Create key" }));

    expect(await findByText(/Pick at least one scope/)).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, 120));
    expect(calls.filter((c) => c.method === "POST")).toHaveLength(0);
  });

  it("rotates a key through the confirmation dialog and reveals the replacement", async () => {
    const { findByText, calls } = await mount(
      routes({ ...keyActionRoutes(issued) }),
    );
    await findByText("hv_live_ab12cd");

    fireEvent.click(screen.getByRole("button", { name: "Rotate" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent(/Rotate hv_live_ab12cd/);
    fireEvent.click(screen.getByRole("button", { name: "Confirm rotate" }));

    const post = await untilCall(
      calls,
      (c) => c.method === "POST" && c.path === "/v1/admin/keys/key-1/rotate",
      "the rotate POST",
    );
    expect(JSON.parse(post.body ?? "{}")).toEqual({});
    expect(await screen.findByText(/hv_live_newsecretvalue/)).toBeInTheDocument();
  });

  it("revokes and suspends through their own routes", async () => {
    const { findByText, calls } = await mount(routes({ ...keyActionRoutes(issued) }));
    await findByText("hv_live_ab12cd");

    fireEvent.click(screen.getByRole("button", { name: "Suspend" }));
    fireEvent.click(screen.getByRole("button", { name: "Confirm suspend" }));
    await untilCall(calls, (c) => c.path === "/v1/admin/keys/key-1/suspend", "the suspend POST");

    fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
    fireEvent.click(screen.getByRole("button", { name: "Confirm revoke" }));
    await untilCall(calls, (c) => c.path === "/v1/admin/keys/key-1/revoke", "the revoke POST");
  });

  it("disables rotate and suspend for a revoked key", async () => {
    const revoked = {
      schema: 1,
      keys: [{ ...keyFixture.keys[0]!, status: "revoked" as const }],
    };
    const { findByText } = await mount(routes({ "/v1/admin/keys": revoked }));
    await findByText("hv_live_ab12cd");
    expect(screen.getByRole("button", { name: "Rotate" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Suspend" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Revoke" })).toBeDisabled();
  });

  it("shows an error state when the key list cannot be read", async () => {
    const { findByText } = await mount(
      routes({ "/v1/admin/keys": new Error("boom") }),
    );
    expect(await findByText("Could not read the keys")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("asks for a tenant before the create form is usable", async () => {
    const { findByText } = await mount(
      routes({
        "/v1/admin/tenants": { schema: 1, tenant_count: 0, key_count: 0, tenants: [] },
      }),
    );
    expect(await findByText("No tenant selected")).toBeInTheDocument();
  });
});
