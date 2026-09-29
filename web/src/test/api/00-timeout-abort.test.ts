import { describe, expect, it, vi } from "vitest";
import { apiGet } from "../../lib/api";
import { neverEndingFetch } from "./helpers";

describe("timeout + abort (§10)", () => {
  it("a wedged GET times out as a normalized ApiError(kind=timeout)", async () => {
    vi.stubGlobal("fetch", neverEndingFetch());
    await expect(apiGet("/health", { timeoutMs: 40, noCache: true })).rejects.toMatchObject({
      kind: "timeout",
    });
  });

  it("an external abort is 'aborted' — never retried", async () => {
    const fetchMock = neverEndingFetch();
    vi.stubGlobal("fetch", fetchMock);
    const ctrl = new AbortController();
    const p = apiGet("/stats", { signal: ctrl.signal, noCache: true });
    ctrl.abort();
    await expect(p).rejects.toMatchObject({ kind: "aborted" });
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
