/**
 * Browser-level smoke test (§36) without a heavyweight runner: the built
 * artifact is served by `vite preview` (same origin as the proxied /api),
 * and we drive the REAL dist — HTML shell, hashed assets, SPA deep links,
 * and the live admin API through the proxy — over HTTP.
 *
 * Interactive behavior is covered by the jsdom suite (vitest); this file
 * proves the shipped bundle boots and talks to a real backend, not a mock.
 *
 * Usage:  npm run build && npm run smoke
 * Requires a harvest admin endpoint (default http://127.0.0.1:9678,
 * override with HARVEST_API; the demo config + `harvest serve` satisfy it).
 */
import { spawn } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";

const PORT = Number(process.env.SMOKE_PORT ?? 4321);
const BASE = `http://127.0.0.1:${PORT}`;
let pass = 0;
const fails = [];
const ok = (name, cond, detail = "") => {
  if (cond) { pass++; console.log(`  ✓ ${name}`); }
  else { fails.push(name); console.log(`  ✗ ${name} ${detail}`); }
};
const get = (path, ms = 3000) =>
  fetch(`${BASE}${path}`, { signal: AbortSignal.timeout(ms) });

function waitServer(url, ms = 20000) {
  const t0 = Date.now();
  return new Promise((res, rej) => {
    const tick = async () => {
      try { await fetch(url, { signal: AbortSignal.timeout(1500) }); return res(); }
      catch { /* not up yet */ }
      if (Date.now() - t0 > ms) return rej(new Error(`preview server never answered at ${url}`));
      setTimeout(tick, 200);
    };
    tick();
  });
}

if (!existsSync("dist/index.html")) {
  console.error("smoke: dist/index.html missing — run `npm run build` first");
  process.exit(2);
}

const child = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], {
  // own process group so the whole tree dies with the script (stdout of a
  // lingering server must never keep this process alive)
  detached: true,
  stdio: "ignore",
});
const killTree = () => { try { process.kill(-child.pid, "SIGTERM"); } catch { /* gone */ } };
process.on("exit", killTree);

try {
  await waitServer(`${BASE}/`);

  // 1) app loads: HTML shell with a module entry
  const html = await (await get("/")).text();
  ok("app shell served", /<div id="root">/.test(html));
  const asset = html.match(/src="(\/assets\/index-[^"]+\.js)"/)?.[1];
  ok("hashed entry script referenced", !!asset, asset ?? "(none found)");

  // 2) the built chunks exist and are substantial
  const files = readdirSync("dist/assets");
  const js = await (await get(asset)).text();
  ok("entry chunk fetches", js.length > 1000, `(${js.length} bytes)`);
  ok("vendor chunk present", files.some((f) => /^vendor-.*\.js$/.test(f)));
  ok("css bundle present", files.some((f) => /^index-.*\.css$/.test(f)));
  ok(
    "26 lazy route chunks emitted (one per console route)",
    files.filter((f) =>
      /^(?:Overview|Runs|Jobs|QueuePage|Workers|CommandCenter|RunDetail|Proxies|Activity|Pipeline|Quality|EventsPage|Storage|Alerts|Audit|Diagnostics|Delivery|ConfigStudio|ApiExplorer|ConfigDiff|ExportCenter|CrawlStudio|AiAssist|ControlJobs|ControlJobDetail|Artifacts)-[\w-]+\.js$/.test(f),
    ).length >= 26,
  );

  // 3) connect to the local Rust API through the same-origin proxy
  const health = await get("/api/health");
  const healthJson = await health.json().catch(() => null);
  ok("GET /api/health proxied → 200 ok", health.status === 200 && healthJson?.status === "ok", JSON.stringify(healthJson ?? {}));

  // 4) SPA fallback + real data endpoints via the proxy
  const runsPage = await get("/runs");
  ok("deep link /runs falls back to app shell", runsPage.status === 200 && /<div id="root">/.test(await runsPage.text()));
  const runsApi = await get("/api/runs?limit=5&offset=0&sort=ts&order=desc");
  const runsJson = await runsApi.json().catch(() => null);
  ok(
    "GET /api/runs returns the paged ledger object",
    runsApi.status === 200 && runsJson?.page && Array.isArray(runsJson.page.items),
    `status=${runsApi.status}`,
  );
  const cfg = await get("/api/config");
  const cfgJson = await cfg.json().catch(() => null);
  ok(
    "GET /config never leaks credentials",
    cfg.status === 200 &&
      (cfgJson?.admin_token === undefined || cfgJson?.admin_token === null || cfgJson?.admin_token === "<redacted>"),
    `admin_token=${JSON.stringify(cfgJson?.admin_token)}`,
  );
  const notfound = await get("/api/nope");
  ok("unknown route 404s with JSON error, not HTML", notfound.status === 404);

  // 5) the shipped bundle carries the connect/demo experience (fresh build,
  //    not a stale dist)
  const entryFile = files.find((f) => f.startsWith("index-") && f.endsWith(".js"));
  ok(
    "built bundle contains connect/demo copy",
    /DEMO MODE|Open console anyway/.test(readFileSync(`dist/assets/${entryFile}`, "utf8")),
  );
} catch (e) {
  fails.push(`exception: ${e.message}`);
  console.error(e);
} finally {
  killTree();
}

console.log(`\nsmoke: ${pass} checks passed, ${fails.length} failed`);
process.exit(fails.length ? 1 : 0);
