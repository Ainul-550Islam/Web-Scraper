//! OpenAPI contract: the served document, the checked-in copy and the router
//! must agree in both directions.
//!
//! `docs/openapi.json` is generated, not hand-written, and the generator is the
//! same code path `/v1/openapi.json` uses. This file is the drift test that
//! keeps the two byte-identical, and that keeps every route the router serves
//! inside the published document.

use std::collections::BTreeSet;

use harvest::config::ManagedApiCfg;
use harvest::scrape_api::models::RESPONSE_SCHEMA;
use harvest::scrape_api::openapi;
use serde_json::{json, Value};

fn read(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e} — the contract test needs it"))
}

/// The `("METHOD", "/v1/...")` arms of `managed_route` in `src/admin.rs`,
/// minus the operator surface.
/// Both the router (`:id`) and OpenAPI (`{job_id}`) spell a parameterised
/// segment differently, and the parameter names differ too. Compare routes
/// in one canonical vocabulary where every parameter is a single placeholder,
/// so the two sides are judged on the shape they share.
fn canonical_route(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
                out.push_str(":param");
            }
            ':' if out.ends_with('/') => {
                // The separator is already in `out`; consume the parameter
                // name and leave the next segment to the main loop.
                while let Some(&c) = chars.peek() {
                    if c == '/' {
                        break;
                    }
                    chars.next();
                }
                out.push_str(":param");
            }
            _ => out.push(c),
        }
    }
    out
}

/// The document's paths, keyed canonically so a router route and a document
/// path with different parameter spellings still meet.
fn documented_paths(doc: &Value) -> std::collections::BTreeMap<String, Value> {
    doc["paths"]
        .as_object()
        .expect("the document has paths")
        .iter()
        .map(|(k, v)| (canonical_route(k), v.clone()))
        .collect()
}

fn documented_surface() -> BTreeSet<(String, String)> {
    let src = read("src/admin.rs");
    let mut out = BTreeSet::new();
    for line in src.lines() {
        // The console router has its own `("GET", "/")` arms; only the
        // managed surface is part of this contract.
        if !line.contains("/v1/") {
            continue;
        }
        for method in ["GET", "POST"] {
            let pat = format!("(\"{method}\", \"");
            if let Some(a) = line.find(&pat) {
                // `pat` ends at the opening quote, so the route starts
                // immediately after it.
                let after = &line[a + pat.len()..];
                if let Some(b) = after.find('"') {
                    let route = after[..b].to_string();
                    // The operator surface is console-bearer only and is not
                    // part of the public contract.
                    if route.starts_with("/v1/admin/") {
                        continue;
                    }
                    out.insert((method.to_string(), route));
                }
            }
        }
    }
    out
}

#[test]
fn every_managed_route_is_documented() {
    let doc: Value =
        serde_json::from_str(&openapi::render(&ManagedApiCfg::default())).expect("valid JSON");
    let paths = documented_paths(&doc);
    let surface = documented_surface();
    assert!(!surface.is_empty(), "the scan found no managed routes");
    for (method, path) in &surface {
        let key = canonical_route(path);
        assert!(
            paths.contains_key(&key),
            "route {method} {path} is served but not in the OpenAPI document"
        );
        let operations = paths[&key].as_object().expect("a path object");
        assert!(
            operations.contains_key(method.as_str().to_lowercase().as_str()),
            "route {method} {path} has no {method} operation in the document"
        );
    }
}

#[test]
fn every_documented_route_is_served() {
    let doc: Value =
        serde_json::from_str(&openapi::render(&ManagedApiCfg::default())).expect("valid JSON");
    let served: BTreeSet<(String, String)> = documented_surface()
        .into_iter()
        .map(|(m, r)| (m, canonical_route(&r)))
        .collect();
    for (path, _) in documented_paths(&doc) {
        assert!(
            served.iter().any(|(_, p)| p == &path),
            "the document documents a route the router does not serve"
        );
    }
}

#[test]
fn the_operator_surface_is_not_advertised() {
    // `/v1/admin/*` is reached with the console bearer, not a tenant key, so
    // publishing it in the public contract would advertise the operator
    // surface to every customer.
    let doc: Value =
        serde_json::from_str(&openapi::render(&ManagedApiCfg::default())).expect("valid JSON");
    for path in doc["paths"].as_object().unwrap().keys() {
        assert!(
            !path.starts_with("/v1/admin/"),
            "{path} is an operator route and must not be in the public document"
        );
    }
}

#[test]
fn the_checked_in_copy_matches_the_served_document_byte_for_byte() {
    let served = openapi::render(&ManagedApiCfg::default());
    let checked_in = read("docs/openapi.json");
    assert_eq!(
        served, checked_in,
        "docs/openapi.json is stale — regenerate it from the route table; the \
         served document and the checked-in copy must be identical"
    );
}

#[test]
fn the_document_is_openapi_31_and_self_consistent() {
    let doc: Value =
        serde_json::from_str(&openapi::render(&ManagedApiCfg::default())).expect("valid JSON");
    assert_eq!(doc["openapi"], "3.1.0");
    assert!(doc["info"]["title"].as_str().unwrap().contains("Harvest"));
    assert!(!doc["info"]["version"].as_str().unwrap().is_empty());
    assert_eq!(
        doc["components"]["securitySchemes"]["apiKey"]["type"],
        "http"
    );
    assert_eq!(
        doc["components"]["securitySchemes"]["apiKey"]["scheme"],
        "bearer"
    );

    for (path, operations) in doc["paths"].as_object().unwrap() {
        for (method, op) in operations.as_object().unwrap() {
            if method == "get" && path == "/v1/openapi.json" {
                continue;
            }
            let security = op.get("security").and_then(Value::as_array);
            match security {
                Some(list) => assert!(
                    !list.is_empty(),
                    "{method} {path} declares an empty security list"
                ),
                None => panic!("{method} {path} is published without any security requirement"),
            }
        }
    }
}

#[test]
fn the_document_carries_the_bounds_in_force() {
    let cfg = ManagedApiCfg::default();
    let doc: Value = serde_json::from_str(&openapi::render(&cfg)).expect("valid JSON");
    let extension = doc
        .pointer("/x-harvest-config")
        .expect("the document exposes its own bounds");
    assert_eq!(extension["max_request_bytes"], cfg.max_request_bytes);
    assert_eq!(extension["max_redirects"], cfg.max_redirects);
    assert_eq!(extension["max_url_len"], cfg.max_url_len);
    assert_eq!(extension["jobs"]["max_jobs"], cfg.jobs.max_jobs);
    assert_eq!(extension["request_timeout_ms"], cfg.request_timeout_ms);

    let url = &doc["components"]["schemas"]["ScrapeRequest"]["properties"]["url"];
    assert_eq!(url["type"], "string");
    assert_eq!(url["format"], "uri");
    assert_eq!(url["maxLength"], cfg.max_url_len);

    // Every payload carries a `schema` discriminator, so a client can pin
    // against a response shape it understands.
    for name in ["Error", "ScrapeResponse", "JobResponse"] {
        let discriminator = &doc["components"]["schemas"][name]["properties"]["schema"];
        assert_eq!(
            discriminator["enum"],
            json!([RESPONSE_SCHEMA]),
            "{name} carries a schema discriminator"
        );
    }
}

#[test]
fn the_document_is_deterministic() {
    let cfg = ManagedApiCfg::default();
    assert_eq!(openapi::render(&cfg), openapi::render(&cfg));
}
