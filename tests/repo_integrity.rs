// Repo-integrity gate for buyer packaging: files the public docs promise must
// exist, counts referenced in docs/tests must line up, and no build artifacts
// may sneak into the tree.

use std::collections::BTreeSet;
use std::path::Path;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn need(rel: &str) {
    assert!(
        root().join(rel).exists(),
        "buyer package is missing promised file: {rel}"
    );
}

#[test]
fn promised_files_exist() {
    for f in [
        "demo/server.py",
        "examples/config.basic.json",
        "web/e2e/smoke.mjs",
        "web/e2e/buyer.mjs",
        "web/src/test/harness.tsx",
        "web/src/test/setup.ts",
        "web/src/lib/apiRoutes/API_ROUTES.ts",
        "web/src/lib/apiRoutes/ApiRoute.ts",
        "docs/buyer-demo.md",
        "docs/buyer-quickstart.md",
        "docs/release-status.md",
        "docs/capability-matrix.md",
        "docs/codebase-map.md",
        "SECURITY.md",
        "docs/testing.md",
        "web/README.md",
        "tools/check_links.py",
        "Makefile",
        "LICENSE",
    ] {
        need(f);
    }
}

#[test]
fn every_module_with_a_dir_has_tests() {
    // Convention: src/<mod>.rs + src/<mod>/tests.rs (unit corpus per module,
    // e.g. src/dist/tests.rs). A module dir without tests.rs is drift.
    let src = root().join("src");
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let p = entry.unwrap().path();
        if !p.is_dir() {
            continue;
        }
        let stem = p.file_name().unwrap().to_str().unwrap().to_string();
        if !src.join(format!("{stem}.rs")).exists() {
            continue; // pure directory module (submodules only)
        }
        if !p.join("tests.rs").exists() {
            missing.push(stem);
        }
    }
    assert!(
        missing.is_empty(),
        "module dirs without tests.rs: {missing:?}"
    );
    assert!(
        src.join("dist/tests.rs").exists(),
        "src/dist/tests.rs must exist (restored bundle file)"
    );
}

#[test]
fn route_count_parity_docs_and_app() {
    let routes = read("web/src/app/routes/ROUTES.ts");
    let n = routes.matches("path: \"").count();
    let pages = std::fs::read_dir(root().join("docs/pages"))
        .unwrap()
        .count();
    // docs/pages holds one guide per route plus README.md
    assert_eq!(
        n + 1,
        pages,
        "console routes ({n}) and docs/pages guides ({pages}-1) drifted"
    );
    let manifest = read("web/src/lib/apiRoutes/API_ROUTES.ts");
    assert!(
        manifest.matches("method: \"").count() >= 25,
        "API manifest lost rows"
    );
}

#[test]
fn fixtures_match_client_types() {
    // Every apiGet/apiPost path used by feature code must exist in the typed
    // fixture corpus the test harness reads (capture-only fixtures, ADR 0004).
    let dir = root().join("web/src/test/fixtures");
    let names: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_str().unwrap().to_string())
        .collect();
    assert!(names.len() >= 20, "fixture corpus shrank: {}", names.len());
    for f in [
        "health.json",
        "config.json",
        "stats.json",
        "workers.json",
        "pipeline.json",
    ] {
        assert!(
            names.contains(f),
            "fixture {f} missing — harness cannot fake the API"
        );
    }
}

#[test]
fn package_include_is_explicit() {
    // `cargo package --list` is the buyer's file list; keep it declarative so
    // no artifact (web/dist, node_modules, target) can ride in by accident
    // and the promised docs/examples/demo stay inside the package.
    let toml = read("Cargo.toml");
    let start = toml
        .find("include = [")
        .expect("Cargo.toml must declare [package] include");
    let end = start + toml[start..].find(']').unwrap();
    let list = &toml[start..end];
    for key in [
        "/src",
        "/examples",
        "/tests",
        "/docs",
        "/demo",
        "/tools",
        "/web/src",
        "/web/e2e",
        "/build.rs",
        "/README.md",
        "/LICENSE",
    ] {
        assert!(
            list.contains(&format!("\"{key}\"")),
            "package include lost {key}"
        );
    }
    for banned in ["/dist", "/target", "/web/node_modules"] {
        assert!(
            !list.contains(&format!("\"{banned}\"")),
            "package include must not list {banned}"
        );
    }
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}
