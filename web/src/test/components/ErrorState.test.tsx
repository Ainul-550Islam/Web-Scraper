import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { ErrorState } from "../../components/ui";
import { ApiError } from "../../lib/api";

describe("ErrorState (components/ui/ErrorState.tsx)", () => {
  it("maps slowdown to the honest backoff copy", () => {
    const { getByText } = render(
      <ErrorState error={new ApiError("slowdown", "too many polls")} />,
    );
    expect(getByText(/429.*polling backs off automatically/si)).toBeTruthy();
  });
  it("maps timeout to the idempotent-retry copy", () => {
    const { getByText } = render(<ErrorState error={new ApiError("timeout", "x")} />);
    expect(getByText(/all console reads are idempotent/i)).toBeTruthy();
  });
  it("shows HTTP status and requestId when present, plus working Retry", () => {
    const onRetry = vi.fn();
    const { getByText, getByRole } = render(
      <ErrorState
        error={new ApiError("server", "engine said no", { status: 503, requestId: "rq-9" })}
        onRetry={onRetry}
      />,
    );
    expect(getByText("HTTP 503")).toBeTruthy();
    expect(getByText("rq-9")).toBeTruthy();
    fireEvent.click(getByRole("button", { name: /Retry/ }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
  it("renders no retry affordance when onRetry is omitted", () => {
    const { queryByRole } = render(<ErrorState error={new Error("boom")} />);
    expect(queryByRole("button", { name: /Retry/ })).toBeNull();
  });
});
