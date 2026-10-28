import { afterEach, describe, expect, it } from "vitest";
import { PAGE_LOADERS } from "../../app/routes";
import { parseHash } from "../../app/router/parseHash";

afterEach(() => { window.location.hash = ""; });

describe("phase 2 routing regressions", () => {
  for (const name of ["constructor", "__proto__", "toString"]) {
    it(`does not treat inherited ${name} as a page component`, () => {
      expect(PAGE_LOADERS[name]).toBeUndefined();
    });
  }
  it("normalizes an unknown hash route to the safe overview", () => {
    window.location.hash = "#/not-a-real-page?tab=private";
    expect(parseHash().path).toBe("/");
  });
  it("normalizes non-route hashes and preserves query values for real routes", () => {
    window.location.hash = "#constructor";
    expect(parseHash().path).toBe("/");
    window.location.hash = "#/runs?run=hello%20world";
    const route = parseHash();
    expect(route.path).toBe("/runs");
    expect(route.query.get("run")).toBe("hello world");
  });
});

import { render, screen } from "@testing-library/react";
import { vi } from "vitest";
import { PageErrorBoundary } from "../../components/PageErrorBoundary";

function BrokenPage(): never { throw new Error("private endpoint/token must not reach the UI"); }

describe("page failure isolation", () => {
  it("renders recovery rather than a blank console and does not reveal the exception", () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      const view = render(<PageErrorBoundary key="bad"><BrokenPage /></PageErrorBoundary>);
      expect(screen.getByRole("alert")).toHaveTextContent("This page could not be loaded");
      expect(document.body.textContent).not.toContain("private endpoint/token");
      view.rerender(<PageErrorBoundary key="good"><h1>Working page</h1></PageErrorBoundary>);
      expect(screen.getByText("Working page")).toBeInTheDocument();
    } finally { log.mockRestore(); }
  });
  it("preserves a literal question mark inside query values", () => {
    window.location.hash = "#/runs?run=a?b&tab=x";
    expect(parseHash().query.get("run")).toBe("a?b");
    expect(parseHash().query.get("tab")).toBe("x");
  });
});
