import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ApiError } from "../../lib/api";
import { ErrorState } from "../../components/ui";

describe("normalized error copy (§33)", () => {
  it("429 → rate-limit guidance, not a stack or body dump", () => {
    render(<ErrorState error={new ApiError("slowdown", "too fast", { status: 429, requestId: "req-1" })} />);
    expect(screen.getByText(/rate limiting the console/i)).toBeInTheDocument();
    expect(screen.getByText("req-1")).toBeInTheDocument();
    expect(screen.getByText(/HTTP 429/)).toBeInTheDocument();
  });

  it("timeout → safe retry message (reads are idempotent)", () => {
    render(<ErrorState error={new ApiError("timeout", "request timed out after 15000 ms")} />);
    expect(screen.getByText("Request timed out")).toBeInTheDocument();
    expect(screen.getByText(/retrying is safe/i)).toBeInTheDocument();
  });

  it("401 → points at the in-memory token, never a raw body", () => {
    render(<ErrorState error={new ApiError("unauthorized", '{"error":', { status: 401 })} />);
    expect(screen.getByText(/bearer token/i)).toBeInTheDocument();
    expect(screen.getByText(/stays in memory/i)).toBeInTheDocument();
  });
});
