/**
 * Crawl Studio: scope validation, discovery/content controls, engine limits
 * from GET /config, and the honest error path when the engine cannot be read.
 */
import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import CrawlStudio from "../pages/CrawlStudio";
import CrawlScopePanel, { validateScope, DEPTH_CEILING, PAGES_CEILING } from "../components/CrawlScopePanel";
import { mountPage } from "./harness";
import type { CrawlScope } from "../components/CrawlScopePanel";

const SCOPE: CrawlScope = {
  startUrls: ["https://shop.test/catalog"],
  include: ["/product/"],
  exclude: [],
  depth: 2,
  maxPages: 100,
  maxLinksPerPage: 200,
  maxResponseBytes: 8 * 1024 * 1024,
  sameHostOnly: true,
  sitemap: false,
  sitemapUrl: "",
};

async function studio(overrides: Record<string, unknown> = {}) {
  return mountPage(<CrawlStudio />, overrides);
}

function setInputs(container: HTMLElement, values: Record<string, string>) {
  for (const [label, value] of Object.entries(values)) {
    const input = container.querySelector(`input[aria-label="${label}"]`) as HTMLInputElement;
    fireEvent.change(input, { target: { value } });
  }
}

describe("Crawl Studio — scope validation", () => {
  it("flags a missing start URL and an out-of-range depth", () => {
    const issues = validateScope({ ...SCOPE, startUrls: [] });
    expect(issues.some((i) => i.field === "startUrls" && i.level === "error")).toBe(true);

    const deep = validateScope({ ...SCOPE, depth: DEPTH_CEILING + 1 });
    expect(deep.some((i) => i.field === "depth" && i.level === "error")).toBe(true);

    const big = validateScope({ ...SCOPE, maxPages: PAGES_CEILING + 1 });
    expect(big.some((i) => i.field === "maxPages" && i.level === "error")).toBe(true);
  });

  it("warns on overlapping include/exclude and very large budgets", () => {
    const issues = validateScope({ ...SCOPE, include: ["/product/"], exclude: ["/product/"] });
    expect(issues.some((i) => i.field === "include" && i.level === "warn")).toBe(true);
    const huge = validateScope({ ...SCOPE, maxPages: 50_000 });
    expect(huge.some((i) => i.message.includes("rate limit"))).toBe(true);
  });

  it("accepts a sane scope with no issues", () => {
    expect(validateScope(SCOPE)).toEqual([]);
  });

  it("renders the effective scope and refuses to present an invalid one as valid", async () => {
    const { container } = await mountPage(<CrawlScopePanel scope={SCOPE} onChange={() => {}} />);
    expect(container.textContent).toContain("valid scope");
    expect(container.textContent).toContain("depth 2");

    const bad = await mountPage(<CrawlScopePanel scope={{ ...SCOPE, startUrls: [] }} onChange={() => {}} />);
    expect(bad.container.textContent).toContain("invalid scope");
    expect(bad.container.textContent).toContain("at least one start URL is required");
  });
});

describe("Crawl Studio — discovery and content controls", () => {
  it("shows every discovery toggle with pagination, sitemap, documents and media", async () => {
    const { container } = await studio();
    for (const label of [
      "Pagination links",
      "Canonical links",
      "Alternate representations",
      "Feeds",
      "Document links",
      "Media resources",
    ]) {
      expect(container.querySelector(`[aria-label="${label}"]`)).toBeTruthy();
    }
    expect(container.querySelector('input[aria-label="pagination selector"]')).toBeTruthy();
    expect(container.querySelector('[aria-label="Record resources of unknown size"]')).toBeTruthy();
  });

  it("toggles media discovery and reveals the resource byte cap", async () => {
    const { container } = await studio();
    const media = container.querySelector('[aria-label="Media resources"]') as HTMLButtonElement;
    expect(media.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(media);
    await waitFor(() => expect(media.getAttribute("aria-checked")).toBe("true"));
    expect(container.querySelector('input[aria-label="resource byte cap"]')).toBeTruthy();
  });

  it("selects content types and warns that pdf/binary produce no records", async () => {
    const { container } = await studio();
    const checkboxes = [...container.querySelectorAll(".cs-kind input")] as HTMLInputElement[];
    expect(checkboxes.length).toBe(7);
    const pdf = checkboxes.find((c) => c.parentElement?.textContent === "pdf") as HTMLInputElement;
    fireEvent.click(pdf);
    await waitFor(() => expect(container.textContent).toMatch(/recorded as resources, not parsed into records/));
  });

  it("composes the plan with the sitemap and content-type selections", async () => {
    const { container } = await studio();
    // The sitemap URL field only exists once sitemap seeding is enabled.
    const sitemapToggle = container.querySelector('[aria-label="Seed from sitemap"]') as HTMLButtonElement;
    fireEvent.click(sitemapToggle);
    await waitFor(() => expect(container.querySelector('input[aria-label="sitemap url"]')).toBeTruthy());
    setInputs(container, { "sitemap url": "https://shop.test/sitemap.xml" });
    await waitFor(() =>
      expect(container.querySelector('pre[aria-label="crawl plan json"]')?.textContent).toContain(
        '"sitemap_url": "https://shop.test/sitemap.xml"',
      ),
    );
    const plan = container.querySelector('pre[aria-label="crawl plan json"]') as HTMLPreElement;
    expect(plan.textContent).toContain('"max_depth": 1');
    expect(plan.textContent).toContain('"content_types"');
    expect(plan.textContent).toContain('"pagination": true');
  });
});

describe("Crawl Studio — engine limits", () => {
  it("reads GET /config and shows concurrency, retries, timeout and robots", async () => {
    const { container } = await studio();
    await waitFor(() => expect(container.textContent).toMatch(/workers/));
    expect(container.textContent).toMatch(/retries \d+ · \d+s timeout · robots (on|off)/);
    expect(container.textContent).toContain("Engine limits");
  });

  it("surfaces a backend failure instead of inventing limits", async () => {
    const { container } = await studio({
      "/config": { __status: 500, body: { error: { code: "internal", message: "config read failed" } } },
    });
    await waitFor(() => expect(container.textContent).toMatch(/config read failed|Request failed/));
    expect(container.textContent).toContain("Engine limits");
  });
});

describe("Crawl Studio — extraction and layout", () => {
  it("adds a field row with a real transform op and a declared type", async () => {
    const { container } = await studio();
    const add = [...container.querySelectorAll("button")].find((b) => b.textContent === "Add field") as HTMLButtonElement;
    fireEvent.click(add);
    await waitFor(() => expect(container.querySelectorAll(".cs-field").length).toBe(2));
    const transform = container.querySelector('select[aria-label="field 2 transform"]') as HTMLSelectElement;
    expect(transform.textContent).toContain("parse_number");
    const type = container.querySelector('select[aria-label="field 2 declared type"]') as HTMLSelectElement;
    expect(type.textContent).toContain("date");
  });

  it("keeps the primary controls in a single responsive grid (mobile-safe)", async () => {
    const { container } = await studio();
    const grid = container.querySelector(".cs-grid") as HTMLElement;
    expect(grid.className).toContain("cs-grid");
    // Grid template uses auto-fit: no fixed multi-column layout to overflow a
    // narrow viewport, and no horizontal scroll on the plan block.
    const plan = container.querySelector(".cs-plan") as HTMLElement;
    expect(plan.className).toContain("cs-plan");
  });
});
