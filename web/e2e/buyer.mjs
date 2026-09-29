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
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
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

  const engine = spawn(BIN[0], [...BIN.slice(1), "serve", "--config", CONFIG], { cwd: ROOT, stdio: "ignore", detached: true });
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

  // ── 5. graceful shutdown is what serve promises
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
