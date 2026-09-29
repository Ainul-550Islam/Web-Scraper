// Route-contract gate: the console's API manifest (web/src/lib/apiRoutes/API_ROUTES.ts)
// must match the actual router arms in src/admin.rs exactly — same methods,
// same paths, same open routes. If either side drifts, CI fails here instead
// of shipping a UI that 404s (or exposes a route the docs never mentioned).

use std::collections::BTreeSet;

fn read(rel: &str) -> String {
    let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e} — contract test needs both files"))
}

/// Collect `("METHOD", "/path")` router arms from admin.rs source.
fn router_routes() -> BTreeSet<(String, String)> {
    let src = read("src/admin.rs");
    let mut out = BTreeSet::new();
    // simple line scan: arms look like `("GET", "/x") => ...` (possibly indented)
    for line in src.lines() {
        let l = line.trim_start();
        for m in ["GET", "POST"] {
            let pat = format!("(\"{m}\", \"/");
            if let Some(a) = l.find(&pat) {
                let after = &l[a + pat.len()..];
                if let Some(b) = after.find('"') {
                    out.insert((m.to_string(), format!("/{}", &after[..b])));
                }
            }
        }
    }
    out
}

/// Collect `{ method: "GET", path: "/x", ... }` rows from the TS manifest.
fn manifest_routes() -> BTreeSet<(String, String)> {
    let src = read("web/src/lib/apiRoutes/API_ROUTES.ts");
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let (Some(m), Some(p)) = (line.find("method: \""), line.find("path: \"")) else {
            continue;
        };
        let method = &line[m + 9..];
        let Some(me) = method.find('"') else { continue };
        let path = &line[p + 7..];
        let Some(pe) = path.find('"') else { continue };
        out.insert((method[..me].to_string(), path[..pe].to_string()));
    }
    out
}

#[test]
fn manifest_matches_router_exactly() {
    let router = router_routes();
    let manifest = manifest_routes();
    assert!(
        !router.is_empty(),
        "no router arms parsed — test regex broken"
    );
    assert_eq!(router.len(), manifest.len(), "route count drift");
    for r in &router {
        assert!(
            manifest.contains(r),
            "router route missing from manifest: {r:?}"
        );
    }
    for m in &manifest {
        assert!(
            router.contains(m),
            "manifest lists a route the router does not have: {m:?}"
        );
    }
}

#[test]
fn open_routes_are_exactly_health_and_live() {
    let admin = read("src/admin.rs");
    assert!(
        admin.contains("route == \"/health\" || route == \"/live\""),
        "auth-open set changed in admin.rs — update manifest auth columns and this test together"
    );
    let manifest = read("web/src/lib/apiRoutes/API_ROUTES.ts");
    for line in manifest.lines() {
        if line.contains("auth: \"always-open\"") {
            assert!(
                line.contains("\"/health\"") || line.contains("\"/live\""),
                "manifest marks a non-open route as always-open: {line}"
            );
        }
    }
}

#[test]
fn mutations_are_only_the_documented_set() {
    // Every POST is a mutation, so the set is pinned: the two operator
    // actions plus the four control-plane submissions (each authenticated,
    // bounded, audited, and idempotent where it matters).
    let routes = manifest_routes();
    let posts: BTreeSet<_> = routes
        .iter()
        .filter(|(m, _)| m == "POST")
        .map(|(_, p)| p.clone())
        .collect();
    assert_eq!(
        posts,
        BTreeSet::from([
            "/alerts/ack".to_string(),
            "/ops/note".to_string(),
            "/control/crawl".to_string(),
            "/control/export".to_string(),
            "/control/ai".to_string(),
            "/control/jobs/:id/cancel".to_string(),
        ])
    );
}

#[test]
fn control_routes_are_never_auth_exempt() {
    let manifest = read("web/src/lib/apiRoutes/API_ROUTES.ts");
    for line in manifest.lines() {
        if line.contains("\"/control/") || line.contains("\"/artifacts/") {
            assert!(
                !line.contains("auth: \"always-open\""),
                "a control-plane route must not be auth-exempt: {line}"
            );
        }
    }
}
