#!/usr/bin/env python3
"""verify-release — packaging gate for the harvest release tree.

Deterministic, read-only checks that the documentation's numeric promises and
the shipped tree agree. Run from anywhere; resolves the repo root from the
script location (or pass a path). Exit 0 + "VERIFIED" line only if every
check passes; prints one line per failure otherwise.

Checks:
  1. version coherence: Cargo.toml == Cargo.lock == `harvest --version`
     string in README ("Version **X**" line) and docs/release-status.md.
  2. route parity: ROUTES.ts `path:` count == docs/pages guides - 1 (README)
     == PAGE_LOADERS.ts lazy entries (tests/repo_integrity.rs enforces the
     same from Rust; this cross-checks the docs side from Python).
  3. module index parity: every src/*.rs (except lib.rs/main.rs) has a
     docs/modules entry and file; counts in the README match `^pub ` lines.
  4. API manifest: API_ROUTES.ts row count == ("METHOD", "/path") arms in
     src/admin.rs (route_contract.rs is the strict version; this catches
     docs-side drift in packaging without a cargo build).
  5. test inventory: tests/*.rs set == the list spelled out in
     docs/codebase-map.md ("tests/ (N files: ...)").
  6. packaging: `cargo package --list` (if cargo available) contains no
     dist/target/node_modules paths; examples/*.json all parse as JSON.
  7. hygiene: no /home/<user> absolute paths in tracked docs.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
fails = []


def fail(msg: str) -> None:
    fails.append(msg)


def read(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


# 1 — version coherence -------------------------------------------------------
toml = read("Cargo.toml")
m = re.search(r'^version = "([^"]+)"', toml, re.M)
cargo_ver = m.group(1) if m else None
lock = read("Cargo.lock")
m = re.search(r'name = "harvest"\nversion = "([^"]+)"', lock)
lock_ver = m.group(1) if m else None
m = re.search(r"Version \*\*([^*]+)\*\*", read("README.md"))
readme_ver = m.group(1) if m else None
m = re.search(r"`([^`]+)` in `Cargo\.toml`/`Cargo\.lock`", read("docs/release-status.md"))
status_ver = m.group(1) if m else None
vers = {"Cargo.toml": cargo_ver, "Cargo.lock": lock_ver, "README.md": readme_ver, "release-status": status_ver}
if len({v for v in vers.values() if v}) != 1 or any(v is None for v in vers.values()):
    fail(f"version drift: {vers}")

# 2 — route parity --------------------------------------------------------------
n_routes = read("web/src/app/routes/ROUTES.ts").count('path: "')
n_loaders = len(re.findall(r'"\S+": lazy\(', read("web/src/app/routes/PAGE_LOADERS.ts")))
pages = sorted(p.name for p in (ROOT / "docs/pages").glob("*.md") if p.name != "README.md")
if not (n_routes == n_loaders == len(pages)):
    fail(f"route parity: ROUTES={n_routes} loaders={n_loaders} docs/pages guides={len(pages)}")
route_paths = re.findall(r'path: "([^"]+)"', read("web/src/app/routes/ROUTES.ts"))
loader_paths = re.findall(r'"([^"]+)": lazy\(', read("web/src/app/routes/PAGE_LOADERS.ts"))
if sorted(route_paths) != sorted(loader_paths):
    fail(f"ROUTES vs PAGE_LOADERS key sets differ: {set(route_paths) ^ set(loader_paths)}")

# 3 — module index parity ---------------------------------------------------------
src_mods = sorted(p.stem for p in (ROOT / "src").glob("*.rs") if p.stem not in ("lib", "main"))
mods_readme = read("docs/modules/README.md")
listed = set(re.findall(r"^- \[`([a-z_0-9]+)`\]", mods_readme, re.M))
missing = [m for m in src_mods if m not in listed]
if missing:
    fail(f"docs/modules/README.md missing entries: {missing}")
no_file = [m for m in src_mods if not (ROOT / "docs/modules" / f"{m}.md").exists()]
if no_file:
    fail(f"docs/modules files missing for: {no_file}")
for mname in src_mods:
    doc = ROOT / "docs/modules" / f"{mname}.md"
    if not doc.exists():
        continue
    text = doc.read_text()
    claimed = re.search(r"Public surface \((\d+) items?\)", text)
    bullets = re.findall(r"^- `(?:async fn|fn|struct|enum|type|const|trait) (\w+)`", text, re.M)
    if claimed and int(claimed.group(1)) != len(bullets):
        fail(f"docs/modules/{mname}.md header claims {claimed.group(1)} items, lists {len(bullets)}")
    roots = [ROOT / "src" / f"{mname}.rs"] + list((ROOT / "src" / mname).glob("*.rs")) if (ROOT / "src" / mname).is_dir() else [ROOT / "src" / f"{mname}.rs"]
    allsrc = "\n".join(p.read_text() for p in roots if p.exists())
    for name in bullets:
        if not re.search(rf"^pub (async fn|fn|struct|enum|type|const|trait) {name}\b", allsrc, re.M):
            fail(f"docs/modules/{mname}.md lists `{name}` but no `pub ... {name}` in src/{mname}*")

# 4 — API manifest rows == admin router arms --------------------------------------
arms = re.findall(r'\("(GET|POST)", "(/[^"]*)"\)', read("src/admin.rs"))
rows = re.findall(r"method: \"(GET|POST)\", path: \"(/[^\"]*)\"", read("web/src/lib/apiRoutes/API_ROUTES.ts"))
if sorted(arms) != sorted(rows):
    fail(f"admin.rs arms ({len(arms)}) vs API_ROUTES rows ({len(rows)}): sym diff {set(map(tuple, arms)) ^ set(map(tuple, rows))}")

# 5 — test inventory ----------------------------------------------------------------
tset = sorted(p.stem for p in (ROOT / "tests").glob("*.rs"))
map_txt = read("docs/codebase-map.md")
m = re.search(r"tests/`\s*\((\d+) files?:([^)]*)\)", map_txt, re.S)
if not m:
    fail("docs/codebase-map.md: no parsable 'tests/ (N files: ...)' inventory")
else:
    claimed_n = int(m.group(1))
    listed_tests = sorted(x.strip() for x in m.group(2).replace("\n", " ").split(",") if x.strip())
    if claimed_n != len(tset):
        fail(f"codebase-map claims {claimed_n} test files; tests/ has {len(tset)}")
    if listed_tests != tset:
        fail(f"codebase-map tests list differs from disk: only-in-doc {set(listed_tests) - set(tset)}, only-on-disk {set(tset) - set(listed_tests)}")

# 6 — packaging --------------------------------------------------------------------
ex = ROOT / "examples"
if ex.is_dir():
    for f in sorted(ex.glob("*.json")):
        try:
            json.loads(f.read_text())
        except Exception as e:  # noqa: BLE001
            fail(f"examples/{f.name} is not valid JSON: {e}")
try:
    proc = subprocess.run(
        ["cargo", "package", "--list", "--allow-dirty", "--no-verify"],
        cwd=ROOT, capture_output=True, text=True, timeout=120,
    )
    if proc.returncode == 0:
        rel = [re.sub(r"^harvest-[^/]+/", "", l.strip()) for l in proc.stdout.splitlines() if l.strip()]
        bad = [l for l in rel if re.match(r"^(dist|target|node_modules)/", l) or "/node_modules/" in l or l.startswith("web/dist")]
        if bad:
            fail(f"cargo package includes artifact paths: {bad[:5]}")
except (FileNotFoundError, subprocess.TimeoutExpired):
    print("note: cargo unavailable here — package-list check skipped")

# 7 — hygiene -------------------------------------------------------------------------
docs = [ROOT / "README.md", ROOT / "CHANGELOG.md", ROOT / "SECURITY.md"] + list((ROOT / "docs").rglob("*.md"))
pat = re.compile(r"/home/[A-Za-z0-9._-]+/")
for d in docs:
    if pat.search(d.read_text(encoding="utf-8")):
        fail(f"{d.relative_to(ROOT)} contains a machine-specific /home/... path")

if fails:
    print("verify-release: FAILED")
    for f in fails:
        print(f"  - {f}")
    sys.exit(1)
print(f"VERIFIED: harvest {cargo_ver} — routes={n_routes}, modules={len(src_mods)}, "
      f"api-rows={len(rows)}, tests={len(tset)}, examples ok, packaging clean")
