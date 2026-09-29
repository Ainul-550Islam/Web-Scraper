import { describe, expect, it, vi } from "vitest";
import type { ApiError} from "../../lib/api";
import { apiGet, auth, setApiBase } from "../../lib/api";
import { jsonResponse } from "./helpers";

describe("security surface (§31)", () => {
  it("token rides in headers only — never the URL, never an error message", async () => {
    const seen: Array<{ url: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        seen.push({ url: String(input), init });
        return jsonResponse({ error: { code: "unauthorized", message: "missing bearer" }, request_id: "r1" }, 401);
      }) as unknown as typeof fetch,
    );
    auth.set("topsecret.tok-42");
    const err = (await apiGet("/runs?limit=1", { noCache: true }).catch((x: unknown) => x)) as ApiError;
    expect((err as ApiError).kind).toBe("unauthorized");
    const h = (seen[0]!.init?.headers ?? {}) as Record<string, string>;
    expect(h.authorization).toBe("Bearer topsecret.tok-42");
    const everything = JSON.stringify([seen[0]!.url, err!.message, (err as ApiError).code]);
    expect(everything).not.toContain("topsecret");
  });

  it("setApiBase persists the endpoint only — credentials stay out of storage", () => {
    auth.set("nope-token");
    setApiBase("/api2");
    expect(localStorage.getItem("harvest.console.api")).toBe("/api2");
    const all = Object.keys(localStorage).map((k) => `${k}=${localStorage.getItem(k)}`).join(";");
    expect(all).not.toContain("nope-token");
    expect(all).not.toContain("console.api\", \"Bearer");
    localStorage.removeItem("harvest.console.api");
    setApiBase(null, false);
  });
});
