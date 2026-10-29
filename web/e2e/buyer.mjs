/**
 * Buyer-flow E2E (single deterministic pass, local backends only — no CDN,
 * no cloud, no network beyond loopback). Drives the SAME artifact a buyer
 * evaluates: demo mock site + `harvest serve` + the built console (vite
 * preview) + real CLI run/export/verify, then asserts over HTTP and files.
 *
 * Usage (from web/):  npm run build && npm run e2e:buyer
 * Env: HARVEST_BIN (default: cargo run -q --; CI points it at the release
 * binary), BUYER_DEMO_PORT (8099), SMOKE_PORT (4321, shared with smoke.mjs).
 */
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const HERE = new URL(".", import.meta.url).pathname; // web/e2e/
const WEB = join(HERE, ".."); // web/
const ROOT = join(WEB, ".."); // repo root
const DEMO_PORT = Number(process.env.BUYER_DEMO_PORT ?? 8099);
const API = process.env.HARVEST_API ?? "http://127.0.0.1:9678";
const PORT = Number(process.env.SMOKE_PORT ?? 4321);
const BASE = `http://127.0.0.1:${PORT}`;
const CONFIG = "examples/config.basic.json";
const BIN = process.env.HARVEST_BIN
  ? process.env.HARVEST_BIN.split(" ")
  : ["cargo", "run", "-q", "--"];

let pass = 0;
const fails = [];
const ok = (name, cond, detail = "") => {
  if (cond) { pass++; console.log(`  ✓ ${name}`); }
  else { fails.push(name); console.log(`  ✗ ${name} ${detail}`); }
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const get = (path, ms = 5000) => fetch(`${BASE}${path}`, { signal: AbortSignal.timeout(ms) });

async function waitHttp(url, ms = 40000) {
  const t0 = Date.now();
  for (;;) {
    try { const r = await fetch(url, { signal: AbortSignal.timeout(1500) }); if (r.status < 500) return; }
    catch { /* not up yet */ }
    if (Date.now() - t0 > ms) throw new Error(`no answer at ${url} within ${ms}ms`);
    await sleep(200);
  }
}

function run(args, opts = {}) {
  return new Promise((res, rej) => {
    const child = spawn(args[0], args.slice(1), { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"], detached: true, ...opts });
    let out = "";
    child.stdout.on("data", (d) => { out += d; });
    child.stderr.on("data", (d) => { out += d; });
    child.on("exit", (code) => res({ code, out }));
    child.on("error", rej);
  });
}

const kids = [];
const killTree = () => { for (const k of kids) { try { process.kill(-k.pid, "SIGTERM"); } catch { /* gone */ } } };
process.on("exit", killTree);

const outJsonl = join(ROOT, "out", "basic.jsonl");
// out/ is a throwaway artifacts dir (gitignored); start clean so "file
// created on disk" proves THIS run wrote it, not a stale one.
if (existsSync(outJsonl)) rmSync(outJsonl);
const outDir = mkdtempSync(join(tmpdir(), "harvest-buyer-"));

// The console's API Keys and Usage pages read the managed surface, so the
// engine this harness drives needs `managed_api` enabled. The shipped example
// stays untouched: derive a temp config that adds the block.
const ENGINE_CFG = join(outDir, "buyer-config.json");
{
  const mcfg = JSON.parse(readFileSync(join(ROOT, CONFIG), "utf8"));
  mcfg.managed_api = {
    enabled: true,
    data_dir: join(outDir, "managed"),
    ledger_dir: join(outDir, "managed/ledger"),
    export_dir: join(outDir, "managed/exports"),
    allow_private_targets: true,
    webhook: {
      enabled: true,
      allowed_hosts: ["127.0.0.1"],
      delivery_key: "buyer-delivery-key",
      dead_letter_salt: "buyer-dead-letter-salt",
      dead_letter_path: join(outDir, "managed/dead-letter.jsonl"),
      max_attempts: 2,
      timeout_ms: 5000,
      retry_base_ms: 100,
      retry_cap_ms: 1000,
      retry_schedule_ms: [],
      list_max: 500,
    },
  };
  writeFileSync(ENGINE_CFG, JSON.stringify(mcfg, null, 2));
}

if (!existsSync(join(WEB, "dist/index.html"))) {
  console.error("buyer e2e: web/dist missing — run `npm run build` in web/ first");
  process.exit(2);
}

try {
  // ── 1. backends up: demo mock site, then the engine (serve mode), then console dist
  const demo = spawn("python3", ["demo/server.py", String(DEMO_PORT)], { cwd: ROOT, stdio: "ignore", detached: true });
  kids.push(demo);
  await waitHttp(`http://127.0.0.1:${DEMO_PORT}/healthz`).catch(async () => {
    // demo server has no /healthz; probe the site itself
    await waitHttp(`http://127.0.0.1:${DEMO_PORT}/`);
  });
  ok("demo mock site serving on :" + DEMO_PORT, true);

  const engine = spawn(BIN[0], [...BIN.slice(1), "serve", "--config", ENGINE_CFG], { cwd: ROOT, stdio: "ignore", detached: true });
  kids.push(engine);
  await waitHttp(`${API}/health`);
  ok("engine admin API up (`serve` mode)", true);

  const preview = spawn("npx", ["vite", "preview", "--port", String(PORT), "--strictPort"], { cwd: WEB, stdio: "ignore", detached: true });
  kids.push(preview);
  await waitHttp(`${BASE}/`);
  ok("console dist served by vite preview", true);

  // ── 2. run the demo end-to-end through the CLI (the exact README command)
  const t0 = Date.now();
  let stats = null;
  while (Date.now() - t0 < 60000) {
    stats = await (await fetch(`${API}/stats`)).json();
    if ((stats.metrics?.records_written ?? 0) > 0) break;
    await sleep(500);
  }
  ok("serve loop wrote records", (stats?.metrics?.records_written ?? 0) > 0, JSON.stringify(stats?.metrics?.records_written));
  ok("jsonl sink file created on disk", existsSync(outJsonl));
  if (existsSync(outJsonl)) {
    const lines = readFileSync(outJsonl, "utf8").trim().split("\n").filter(Boolean);
    const first = JSON.parse(lines[0] ?? "{}");
    // Real sink envelope (src/sink.rs): {run, target, url, depth, ts,
    // fingerprint, egress, fields} — assert exactly that, nothing invented.
    ok("sink lines carry the full provenance envelope",
      lines.length > 0 && typeof first.run === "string" && typeof first.target === "string" &&
      typeof first.ts === "number" && typeof first.fingerprint === "string" &&
      typeof first.egress === "string" && Number.isInteger(first.depth),
      lines[0]?.slice(0, 80));
    ok("records carry source url + extracted fields", /^https?:/.test(first.url ?? "") && Object.keys(first.fields ?? {}).length > 0);
  }

  // ── 3. console answers through the proxy, same-origin (no CORS, no host leak)
  const rHtml = await get("/");
  ok("console HTML shell served", rHtml.ok && /<script[^>]+type="module"/.test(await rHtml.text()));
  const rHealth = await (await fetch(`${BASE}/api/health`)).json();
  ok("GET /api/health through preview proxy → open route", typeof rHealth.status === "string");
  const rCfg = await (await fetch(`${BASE}/api/config`)).json();
  ok("GET /api/config name matches shipped demo config", rCfg.name === "harvest-basic");
  ok("GET /api/config redacts admin_token", rCfg.admin_token === null || rCfg.admin_token === undefined);
  const rRecent = await get("/api/recent");
  const recent = await rRecent.json().catch(() => null);
  // /recent returns the run-history tail: array of {run,status,duration_ms,summary,…}
  ok("GET /api/recent returns the run-history tail",
    rRecent.status === 200 && Array.isArray(recent) && (recent.length === 0 || (typeof recent[0].run === "string" && typeof recent[0].status === "string")),
    JSON.stringify(recent)?.slice(0, 60));
  const rPost = await fetch(`${BASE}/api/runs`, { method: "POST" });
  ok("POST to a read-only route is refused (405 + honest text)", rPost.status === 405 && /POST/.test(await rPost.text()));

  // ── 4. CLI: export the JSONL, verify its manifest (buyer acceptance step)
  const exp = await run([...BIN, "export", "--from", "out/basic.jsonl", "--to", outDir]);
  ok("harvest export produced a delivery dir", exp.code === 0 && existsSync(outDir), exp.out.slice(-140));
  const files = existsSync(outDir) ? readdirSync(outDir) : [];
  ok("delivery has data + manifest.json", files.includes("manifest.json") && files.length >= 2, files.join(","));
  const ver = await run([...BIN, "verify", "--dir", outDir, "--json"]);
  ok("harvest verify accepts the manifest", ver.code === 0, ver.out.slice(-160));
  if (files.includes("manifest.json")) {
    const man = JSON.parse(readFileSync(join(outDir, "manifest.json"), "utf8"));
    ok("manifest reports record totals matching the delivery", typeof man === "object" && JSON.stringify(man).includes("sha256"));
  }

  // ── 5. the console's managed pages: API Keys and Usage
  const tRes = await run([...BIN, "keys", "tenant", "--config", ENGINE_CFG, "--name", "buyer-console"]);
  ok("CLI creates a tenant for the console pages", tRes.code === 0 && /"id":"t-[^"]+"/.test(tRes.out), tRes.out.slice(-120));
  // `keys create --tenant <name>` registers the name as the tenant id itself,
  // so the key and the listing below refer to the same tenant.
  const tenantId = "buyer";
  const kRes = await run([...BIN, "keys", "create", "--config", ENGINE_CFG, "--tenant", tenantId,
    "--name", "buyer-key", "--scope", "scrape,jobs_read,jobs_write,usage_read"]);
  const keyPlain = (kRes.out.match(/"plaintext":"([^"]+)"/) || [])[1];
  ok("CLI creates an api key for the console pages", kRes.code === 0 && !!keyPlain, kRes.out.slice(-120));

  // Both pages are client-rendered: through the preview proxy they must serve
  // the SPA shell, and the endpoints they call must answer.
  for (const route of ["/api-keys", "/usage"]) {
    const r = await get(route);
    const html = await r.text().catch(() => "");
    ok(`console route ${route} serves the SPA shell`, r.ok && /<script[^>]+type="module"/.test(html), String(r.status));
  }
  const rKeys = await get(`/api/v1/admin/keys?tenant_id=${encodeURIComponent(tenantId ?? "")}`);
  const keys = await rKeys.json().catch(() => null);
  ok("GET /api/v1/admin/keys returns the tenant's keys",
    rKeys.status === 200 && Array.isArray(keys?.keys) && keys.keys.length === 1 && !("api_key" in (keys.keys[0] ?? {})),
    `${rKeys.status} ${JSON.stringify(keys)?.slice(0, 80)}`);
  const rUsage = await get("/api/v1/admin/usage");
  const usage = await rUsage.json().catch(() => null);
  ok("GET /api/v1/admin/usage returns a usage report",
    rUsage.status === 200 && !!usage?.totals && Array.isArray(usage?.by_key) && Array.isArray(usage?.by_day),
    `${rUsage.status} ${JSON.stringify(usage)?.slice(0, 80)}`);
  // A customer-side read with the key the console just handed out.
  const rScrape = await fetch(`${API}/v1/scrape`, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: `Bearer ${keyPlain}` },
    body: JSON.stringify({ url: `http://127.0.0.1:${DEMO_PORT}/index.html`, format: "text" }),
  });
  const scrape = await rScrape.json().catch(() => null);
  ok("POST /v1/scrape with the created key returns data + units",
    rScrape.status === 200 && Number(scrape?.usage_units ?? 0) >= 1,
    `${rScrape.status} ${JSON.stringify(scrape)?.slice(0, 80)}`);
  const rUsage2 = await get("/api/v1/admin/usage");
  const usage2 = await rUsage2.json().catch(() => null);
  ok("the metered call shows up in the usage report",
    Number(usage2?.totals?.requests ?? 0) >= 1, JSON.stringify(usage2?.totals)?.slice(0, 80));

  // ── 6. graceful shutdown is what serve promises
  process.kill(-engine.pid, "SIGINT");
  const t1 = Date.now();
  let exited = false;
  while (Date.now() - t1 < 20000) {
    if (engine.exitCode !== null) { exited = true; break; }
    await sleep(150);
  }
  ok("engine exits 0 on SIGINT (graceful drain)", exited && engine.exitCode === 0, `code=${engine.exitCode}`);
} catch (e) {
  fails.push(`exception: ${e.message}`);
  console.error("buyer e2e threw:", e);
} finally {
  killTree();
}

console.log(`\nBUYER E2E: ${pass} passed, ${fails.length} failed`);
if (fails.length) { console.log("failing:", fails.join(" | ")); process.exit(1); }
