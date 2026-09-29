import { describe, expect, it } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { ApiError } from "../../lib/api";
import { Poller } from "./helpers";

describe("poller backoff on 429 (§10)", () => {
  it("waits at least one extra interval after a slowdown before refetching", async () => {
    const calls: number[] = [];
    const fetcher = async (): Promise<unknown> => {
      calls.push(Date.now());
      if (calls.length === 1) throw new ApiError("slowdown", "too fast", { status: 429 });
      return { ok: true };
    };
    render(<Poller fetcher={fetcher} />);
    await waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2), { timeout: 7000 });
    const gap = calls[1]! - calls[0]!;
    // without backoff the second call would land ~1500 ms after the first
    expect(gap).toBeGreaterThanOrEqual(2800);
    expect(gap).toBeLessThan(6000);
    expect(await screen.findByTestId("state")).toHaveTextContent("live");
  }, 12000);
});
