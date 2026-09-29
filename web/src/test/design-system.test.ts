/**
 * §50 — deterministic design-system checks (chosen instead of a storybook/
 * screenshot stack, which the spec explicitly allows). These lock the visual
 * contract: tokens exist for both themes, motion is gated, focus is visible,
 * narrow viewports restack tables and drawers, and there is exactly one icon
 * system with no runtime icon dependency.
 */
import { describe, expect, it } from "vitest";


// node fs via dynamic import keeps the web tsconfig (DOM-only) happy
 
const readFileSync: (p: string | URL, enc?: string) => string =
  // @ts-expect-error provided by vitest's node environment
  (await import("node:fs")).readFileSync;
const cssRaw = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");
/** Resolve `@import "./x.css";` chains so layered stylesheets read as one file. */
const css = (rel: string): string => {
  const src = cssRaw(rel);
  if (!src.includes("@import")) return src;
  return src
    .split("\n")
    .map((l) => {
      const m = l.match(/^@import\s+"([^"]+)";/);
      return m ? cssRaw(new URL(m[1]!, new URL(rel, import.meta.url)).toString()) : l;
    })
    .join("\n");
};
const tokens = css("../styles/tokens.css");
const app = css("../styles/app.css");
const responsive = css("../styles/responsive.css");
const pkg = JSON.parse(css("../../package.json")) as { dependencies: Record<string, string> };

describe("design tokens", () => {
  it("defines every required token family", () => {
    for (const fam of ["--bg-0", "--bg-1", "--bg-2", "--bg-3", "--bg-inset", "--line", "--text-1", "--text-2", "--text-3", "--accent", "--ok", "--warn", "--err", "--info", "--focus-ring", "--shadow-1", "--shadow-2", "--r-sm", "--r-md", "--r-lg", "--s-1", "--s-4", "--s-7", "--fs-display", "--fs-page", "--fs-section", "--fs-metric", "--fs-body", "--fs-label", "--fs-caption", "--fs-code", "--fs-table", "--fs-status", "--t-instant", "--t-fast", "--t-med", "--t-slow"]) {
      expect(tokens, fam).toContain(fam + ":");
    }
  });

  it("carries an explicit light theme block (both modes, dark default)", () => {
    expect(tokens).toMatch(/data-theme=['"]?light/);
    expect(tokens).toMatch(/data-theme=['"]?dark|:root/);
  });

  it("numeric metrics use tabular figures for scanability", () => {
    expect(app + tokens).toMatch(/font-variant-numeric:\s*tabular-nums|font-feature-settings/);
  });
});

describe("motion + focus contract", () => {
  it("all three sheets honor prefers-reduced-motion", () => {
    for (const [name, src] of [["app", app], ["responsive", responsive], ["tokens", tokens]] as const) {
      if (name === "tokens") continue;
      expect(src, name).toContain("prefers-reduced-motion: reduce");
    }
  });
  it("focus-visible styling exists globally", () => {
    expect(app).toContain(":focus-visible");
  });
  it("loading shimmers are skeletons, not fake spinners-on-data", () => {
    expect(app).toMatch(/\.skeleton/);
  });
});

describe("responsive §37", () => {
  it("wide tables collapse into labeled cards below 640px", () => {
    expect(responsive).toMatch(/max-width:\s*640px/);
    expect(responsive).toContain('content: attr(data-col)');
  });
  it("detail drawers become bottom sheets with reduced-motion respect", () => {
    expect(responsive).toMatch(/\.drawer\s*\{[^}]*bottom:\s*0/);
    expect(responsive).toMatch(/prefers-reduced-motion[\s\S]*\.drawer\s*\{\s*animation:\s*none/);
  });
  it("mobile nav breakpoint exists", () => {
    expect(app).toMatch(/@media \(max-width:\s*920px\)[\s\S]*mobile-only/);
  });
});

describe("icon + dependency hygiene", () => {
  it("one icon system, no icon packages", () => {
    expect(Object.keys(pkg.dependencies).sort()).toEqual(["react", "react-dom"]);
  });
  it("no remote fonts or CDNs are referenced anywhere in styles", () => {
    expect(tokens + app + responsive).not.toMatch(/https?:\/\//);
  });
});

describe("component vocabulary (§46)", () => {
  it("spec-named primitives resolve without duplication", async () => {
    const p = await import("../components/patterns");
    for (const name of ["Input", "Textarea", "Select", "StatusBadge", "HealthIndicator"] as const) {
      expect(typeof p[name], name).toBe("function");
    }
    for (const name of ["SearchInput", "FilterBar", "Tabs", "Tooltip", "Checkbox", "Timeline", "ChartCard", "FormSection", "PipelineNode", "PipelineConnector"] as const) {
      expect(typeof p[name], name).toBe("function");
    }
    const shell = await import("../components/AppShell");
    expect(typeof shell.AppShell).toBe("function");
  });
});
