// Contract: the API Explorer manifest vs what client code actually calls and
// what the Rust router actually has (the other half lives in
// tests/route_contract.rs, which pins manifest ⊇/⊆ admin.rs).

import { describe, expect, it } from "vitest";
// node fs via dynamic import keeps the web tsconfig (DOM-only) happy —
// same convention as design-system.test.ts.
type Fs = {
  readFileSync: (p: string | URL, enc: "utf8") => string;
  readdirSync: (p: string) => string[];
  statSync: (p: string) => { isDirectory(): boolean };
};
const fs = (await import("node:fs")) as unknown as Fs;
const { join } = (await import("node:path")) as unknown as { join: (...p: string[]) => string };
import { API_ROUTES } from "../../lib/apiRoutes";

// vitest runs with cwd = web/ (npm scripts + Makefile guarantee this)
const cwd = (globalThis as unknown as { process: { cwd(): string } }).process.cwd();
const srcRoot = join(cwd, "src");
const docsApi = join(cwd, "..", "docs", "api");

function walk(dir: string, out: string[] = []): string[] {
  for (const e of fs.readdirSync(dir)) {
    const p = join(dir, e);
    if (e === "node_modules" || e === "dist") continue;
    if (fs.statSync(p).isDirectory()) walk(p, out);
    else if (/\.tsx?$/.test(p)) out.push(p);
  }
  return out;
}

describe("API Explorer manifest contract", () => {
  it("has unique method+path rows", () => {
    const keys = API_ROUTES.map((r) => `${r.method} ${r.path}`);
    expect(new Set(keys).size).toBe(keys.length);
    expect(keys.length).toBeGreaterThanOrEqual(25);
  });

  it("marks exactly /health and /live as always-open", () => {
    const open = API_ROUTES.filter((r) => r.auth === "always-open").map((r) => r.path).sort();
    expect(open).toEqual(["/health", "/live"]);
    // the two operator actions plus the four control-plane mutations
    expect(API_ROUTES.filter((r) => r.method === "POST").map((r) => r.path).sort()).toEqual([
      "/alerts/ack",
      "/control/ai",
      "/control/crawl",
      "/control/export",
      "/control/jobs/:id/cancel",
      "/ops/note",
    ]);
  });

  it("every path every client call uses is in the manifest", () => {
    const manifest = new Set(API_ROUTES.map((r) => r.path));
    const files = walk(srcRoot);
    const used = new Set<string>();
    for (const f of files) {
      if (f.includes("lib/apiRoutes") || f.includes(join(srcRoot, "test")) || /\.test\.tsx?$/.test(f)) continue;
      const src = fs.readFileSync(f, "utf8");
      const re = /api(Get|Post|Raw)<[^>]*>\(\s*[`"']([^`"'?]+)/g;
      for (const m of src.matchAll(re)) used.add(m[2]!);
      // ConnectionProvider + api.ts build plain fetches too:
      const re2 = /fetch\(\s*[`"'](?:\$\{API_BASE\}\/?|\.)?\/?([a-z0-9/_-]+)[`"?]/g;
      for (const m of src.matchAll(re2)) if (m[1] && m[1] !== "api") used.add(`/${m[1]}`);
    }
    // dynamic ids: `/control/jobs/${encodeURIComponent(id)}` is a call to
    // `/control/jobs/:id`. Normalize interpolations so the manifest still
    // governs them instead of the check quietly skipping them.
    const missing = [...used]
      .map((p) => p.replace(/\$\{[^}]*\}/g, ":id"))
      .filter((p) => !manifest.has(p));
    expect(missing, `client calls not in API_ROUTES: ${missing.join(", ")}`).toEqual([]);
  });

  it("each route's doc file exists under docs/api", () => {
    for (const r of API_ROUTES) {
      const d = r.doc;
      if (!d) continue;
      expect(() => fs.readFileSync(join(docsApi, d), "utf8"), `missing ${d}`).not.toThrow();
    }
  });
});
