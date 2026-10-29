//! OpenAPI 3.1 document, generated from the route table.
//!
//! The document is built from the same constants the handlers use, so the
//! contract and the implementation cannot drift apart: there is no separate
//! hand-written spec to forget to update. `docs/openapi.json` is a checked-in
//! copy of `render()` and the contract test asserts the two are byte-for-byte
//! identical.

use serde_json::{json, Map, Value};

use crate::config::ManagedApiCfg;
use crate::scrape_api::models::{
    RESPONSE_SCHEMA, WEBHOOK_SIGNATURE_HEADER, WEBHOOK_TIMESTAMP_HEADER,
};

/// The OpenAPI version this document claims.
pub const OPENAPI_VERSION: &str = "3.1.0";

/// The documented request body limit, in bytes.
pub const MAX_REQUEST_BYTES: u64 = crate::config::V1_BODY_LIMIT;

fn error_schema() -> Value {
    json!({
        "type": "object",
        "required": ["schema", "error"],
        "properties": {
            "schema": { "type": "integer", "description": "Response schema version.", "enum": [RESPONSE_SCHEMA] },
            "error": {
                "type": "object",
                "required": ["code", "message"],
                "properties": {
                    "code": { "type": "string" },
                    "message": { "type": "string" },
                    "details": { "type": ["object", "null"] },
                    "retry_after_ms": { "type": ["integer", "null"] },
                },
                "additionalProperties": false,
            },
        },
        "additionalProperties": false,
    })
}

fn chain_schema() -> Value {
    json!({
        "type": "object",
        "required": ["seq", "hash"],
        "properties": {
            "seq": { "type": "integer", "minimum": 0 },
            "hash": { "type": "string" },
        },
        "additionalProperties": false,
    })
}

fn artifact_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id", "kind", "bytes"],
        "properties": {
            "id": { "type": "string" },
            "kind": { "type": "string", "enum": ["html", "screenshot", "json"] },
            "bytes": { "type": "integer", "minimum": 0 },
        },
        "additionalProperties": false,
    })
}

fn scrape_response_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "schema", "request_id", "status", "url", "final_url", "content_type",
            "bytes", "latency_ms", "render", "format", "usage_units", "cost", "chain",
        ],
        "properties": {
            "schema": { "type": "integer", "enum": [RESPONSE_SCHEMA] },
            "request_id": { "type": "string" },
            "status": { "type": "integer" },
            "url": { "type": "string", "format": "uri" },
            "final_url": { "type": "string", "format": "uri" },
            "content_type": { "type": "string" },
            "bytes": { "type": "integer", "minimum": 0 },
            "latency_ms": { "type": "integer", "minimum": 0 },
            "render": { "type": "string", "enum": ["none", "browser"] },
            "format": { "type": "string", "enum": ["html", "text", "markdown", "json", "screenshot"] },
            "body": { "type": ["string", "null"] },
            "extracted": { "type": ["object", "null"] },
            "records": { "type": ["array", "null"], "items": { "type": "object" } },
            "artifacts": { "type": "array", "items": artifact_schema() },
            "usage_units": { "type": "integer", "minimum": 0 },
            "cost": { "type": "string" },
            "screenshot": { "type": ["object", "null"], "allOf": [artifact_schema()] },
            "chain": chain_schema(),
        },
        "additionalProperties": false,
    })
}

fn job_response_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "schema", "job_id", "status", "url", "request_id", "created_at_ms",
            "attempts", "cancelled", "chain",
        ],
        "properties": {
            "schema": { "type": "integer", "enum": [RESPONSE_SCHEMA] },
            "job_id": { "type": "string" },
            "status": {
                "type": "string",
                "enum": ["queued", "running", "done", "failed", "cancelled"],
            },
            "url": { "type": "string", "format": "uri" },
            "request_id": { "type": "string" },
            "scrape": { "type": ["object", "null"], "allOf": [scrape_response_schema()] },
            "records": { "type": "array", "items": { "type": "object" } },
            "artifacts": { "type": "array", "items": artifact_schema() },
            "error": { "type": ["object", "null"] },
            "created_at_ms": { "type": "integer", "minimum": 0 },
            "started_at_ms": { "type": ["integer", "null"] },
            "finished_at_ms": { "type": ["integer", "null"] },
            "attempts": { "type": "integer", "minimum": 0, "maximum": 255 },
            "cancelled": { "type": "boolean" },
            "chain": chain_schema(),
        },
        "additionalProperties": false,
    })
}

fn scrape_request_schema() -> Value {
    json!({
        "type": "object",
        "required": ["url"],
        "properties": {
            "url": {
                "type": "string",
                "format": "uri",
                "maxLength": crate::config::V1_MAX_URL_LEN,
                "description": "Absolute http(s) URL. SSRF-checked before anything else; loopback, private, link-local, multicast and unspecified targets are rejected, and every redirect hop is re-checked.",
            },
            "render": { "type": "string", "enum": ["none", "browser"], "default": "none" },
            "proxy_region": { "type": "string", "maxLength": 64 },
            "session_key": {
                "type": "string",
                "maxLength": 128,
                "pattern": "^[A-Za-z0-9_-]{1,128}$",
                "description": "Stable browser-session key so a repeat call resumes instead of restarting.",
            },
            "extract": {
                "type": "object",
                "properties": {
                    "fields": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "object",
                            "properties": {
                                "css": { "type": "string" },
                                "path": { "type": "string" },
                                "attr": { "type": "string" },
                                "all": { "type": "boolean" },
                                "transforms": { "type": "array" },
                            },
                            "additionalProperties": false,
                        },
                    },
                    "selector": { "type": "object" },
                    "max_records": { "type": "integer", "minimum": 1 },
                },
                "additionalProperties": false,
            },
            "format": {
                "type": "string",
                "enum": ["html", "text", "markdown", "json", "screenshot"],
                "default": "html",
            },
            "screenshot": { "type": "boolean", "default": false },
            "wait": {
                "type": "object",
                "properties": {
                    "ms": { "type": "integer", "minimum": 0 },
                    "until_selector": { "type": "string" },
                    "settle_ms": { "type": "integer", "minimum": 0 },
                },
                "additionalProperties": false,
            },
            "webhook_url": {
                "type": "string",
                "format": "uri",
                "description": "Public https callback. Deliveries are HMAC-SHA256 signed and retried, then dead-lettered.",
            },
            "idempotency_key": { "type": "string", "maxLength": 128 },
        },
        "additionalProperties": false,
    })
}

/// Build the whole document.
pub fn spec(cfg: &ManagedApiCfg) -> Value {
    let mut paths = Map::new();

    paths.insert(
        "/v1/scrape".to_string(),
        json!({
            "post": {
                "operationId": "scrape",
                "summary": "Scrape one URL and return the response.",
                "description": "Runs the full compliance path (robots.txt, per-domain pacing, circuit breakers, refusal handling) and returns the body, extracted fields or records.",
                "security": [{ "apiKey": [] }],
                "parameters": [idempotency_parameter()],
                "requestBody": {
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": scrape_request_schema(),
                        }
                    }
                },
                "responses": {
                    "200": {
                        "description": "The retrieval succeeded.",
                        "content": {
                            "application/json": { "schema": scrape_response_schema() }
                        }
                    },
                    "401": error_response("missing or invalid api key"),
                    "403": error_response("the api key does not have the required scope"),
                    "404": error_response("no api key exists with this value"),
                    "409": error_response("the key exists but is suspended, expired or revoked"),
                    "429": {
                        "description": "Rate limit, concurrency cap or monthly quota exhausted.",
                        "content": {
                            "application/json": { "schema": error_schema() }
                        }
                    },
                    "4xx": error_response("the request was rejected"),
                    "default": error_response("an internal error"),
                }
            }
        }),
    );

    paths.insert(
        "/v1/jobs".to_string(),
        json!({
            "post": {
                "operationId": "createJob",
                "summary": "Schedule a scrape job.",
                "security": [{ "apiKey": [] }],
                "parameters": [idempotency_parameter()],
                "requestBody": {
                    "required": true,
                    "content": {
                        "application/json": {
                            "schema": scrape_request_schema()
                        }
                    }
                },
                "responses": {
                    "202": {
                        "description": "The job was queued.",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "required": ["schema", "job_id", "status", "estimated_units", "idempotency_replayed", "chain"],
                                    "properties": {
                                        "schema": { "type": "integer", "enum": [RESPONSE_SCHEMA] },
                                        "job_id": { "type": "string" },
                                        "status": { "type": "string", "enum": ["queued"] },
                                        "estimated_units": { "type": "integer", "minimum": 0 },
                                        "idempotency_replayed": { "type": "boolean" },
                                        "chain": chain_schema(),
                                    },
                                    "additionalProperties": false,
                                }
                            }
                        }
                    },
                    "400": error_response("the request was rejected"),
                    "401": error_response("missing or invalid api key"),
                    "403": error_response("the api key does not have the required scope"),
                    "409": error_response("the key exists but is suspended, expired or revoked"),
                    "429": error_response("a limit was exhausted"),
                    "default": error_response("an internal error"),
                }
            },
            "get": {
                "operationId": "listJobs",
                "summary": "List this tenant's jobs.",
                "security": [{ "apiKey": [] }],
                "parameters": [
                    {
                        "in": "query",
                        "name": "limit",
                        "schema": { "type": "integer", "minimum": 1, "maximum": crate::config::V1_JOBS_MAX_LIST },
                        "description": "Maximum jobs to return.",
                    }
                ],
                "responses": {
                    "200": {
                        "description": "Jobs for this tenant.",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "required": ["schema", "jobs"],
                                    "properties": {
                                        "schema": { "type": "integer", "enum": [RESPONSE_SCHEMA] },
                                        "jobs": { "type": "array", "items": job_response_schema() },
                                    },
                                    "additionalProperties": false,
                                }
                            }
                        }
                    },
                    "401": error_response("missing or invalid api key"),
                    "403": error_response("the api key does not have the required scope"),
                    "default": error_response("an internal error"),
                }
            }
        }),
    );

    paths.insert(
        "/v1/jobs/{job_id}".to_string(),
        json!({
            "get": {
                "operationId": "getJob",
                "summary": "Read one job.",
                "security": [{ "apiKey": [] }],
                "parameters": [job_id_parameter()],
                "responses": {
                    "200": {
                        "description": "The job, or its terminal result once it has run.",
                        "content": {
                            "application/json": { "schema": job_response_schema() }
                        }
                    },
                    "401": error_response("missing or invalid api key"),
                    "403": error_response("the api key does not have the required scope"),
                    "404": error_response("no job with this id, or it belongs to another tenant"),
                    "default": error_response("an internal error"),
                }
            }
        }),
    );

    paths.insert(
        "/v1/jobs/{job_id}/cancel".to_string(),
        json!({
            "post": {
                "operationId": "cancelJob",
                "summary": "Cancel a queued or running job.",
                "security": [{ "apiKey": [] }],
                "parameters": [job_id_parameter()],
                "responses": {
                    "200": {
                        "description": "Cancellation was requested.",
                        "content": {
                            "application/json": { "schema": job_response_schema() }
                        }
                    },
                    "404": error_response("no job with this id, or it belongs to another tenant"),
                    "default": error_response("an internal error"),
                }
            }
        }),
    );

    paths.insert(
        "/v1/usage".to_string(),
        json!({
            "get": {
                "operationId": "getUsage",
                "summary": "Read the usage ledger for this tenant.",
                "description": "Returns per-key rows, a per-day series and the totals for the requested window.",
                "security": [{ "apiKey": [] }],
                "parameters": [
                    {
                        "in": "query",
                        "name": "key_id",
                        "schema": { "type": "string" },
                        "description": "Restrict to one api key.",
                    },
                    {
                        "in": "query",
                        "name": "since",
                        "schema": { "type": "integer", "minimum": 0 },
                        "description": "Inclusive lower bound, epoch milliseconds.",
                    },
                    {
                        "in": "query",
                        "name": "until",
                        "schema": { "type": "integer", "minimum": 0 },
                        "description": "Inclusive upper bound, epoch milliseconds.",
                    },
                    {
                        "in": "query",
                        "name": "limit",
                        "schema": { "type": "integer", "minimum": 1, "maximum": crate::config::V1_JOBS_MAX_LIST },
                    }
                ],
                "responses": {
                    "200": {
                        "description": "The usage report.",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "required": ["schema", "since_ms", "until_ms", "totals", "by_key", "by_day", "key_count"],
                                    "properties": {
                                        "schema": { "type": "integer", "enum": [RESPONSE_SCHEMA] },
                                        "since_ms": { "type": "integer", "minimum": 0 },
                                        "until_ms": { "type": "integer", "minimum": 0 },
                                        "generated_at_ms": { "type": "integer", "minimum": 0 },
                                        "totals": { "type": "object" },
                                        "by_key": { "type": "array", "items": { "type": "object" } },
                                        "by_day": { "type": "array", "items": { "type": "object" } },
                                        "key_count": { "type": "integer", "minimum": 0 },
                                    },
                                    "additionalProperties": false,
                                }
                            }
                        }
                    },
                    "401": error_response("missing or invalid api key"),
                    "403": error_response("the api key does not have the required scope"),
                    "default": error_response("an internal error"),
                }
            }
        }),
    );

    paths.insert(
        "/v1/openapi.json".to_string(),
        json!({
            "get": {
                "operationId": "getOpenApi",
                "summary": "This document.",
                "description": "Generated from the route table, so the served contract and the checked-in copy cannot drift.",
                "responses": {
                    "200": {
                        "description": "The OpenAPI document.",
                        "content": {
                            "application/json": {
                                "schema": { "type": "object" }
                            }
                        }
                    }
                }
            }
        }),
    );

    let mut security_schemes = Map::new();
    security_schemes.insert(
        "apiKey".to_string(),
        json!({
            "type": "http",
            "scheme": "bearer",
            "description": "A tenant api key, prefixed `hv1_`. Keys are stored only as SHA-256 hashes and are shown once at creation.",
        }),
    );

    let mut components = Map::new();
    components.insert(
        "securitySchemes".to_string(),
        Value::Object(security_schemes),
    );
    components.insert(
        "schemas".to_string(),
        json!({
            "ScrapeRequest": scrape_request_schema(),
            "ScrapeResponse": scrape_response_schema(),
            "JobResponse": job_response_schema(),
            "Error": error_schema(),
            "ChainHead": chain_schema(),
            "ArtifactRef": artifact_schema(),
        }),
    );

    let mut root = Map::new();
    root.insert("openapi".to_string(), json!(OPENAPI_VERSION));
    root.insert(
        "info".to_string(),
        json!({
            "title": "Harvest managed scrape API",
            "version": "1.0.0",
            "description": concat!(
                "Customer-facing scrape API. Every endpoint requires a tenant api key; requests are SSRF-guarded, ",
                "compliance-checked and metered through an append-only hash-chained ledger. ",
                "Request bodies are bounded by the operator's configured limit.",
            ),
        }),
    );
    root.insert("paths".to_string(), Value::Object(paths));
    root.insert("components".to_string(), Value::Object(components));
    root.insert("x-harvest-config".to_string(), config_summary(cfg));
    root.insert(
        "servers".to_string(),
        json!([
            {
                "url": "/",
                "description": "Same origin as the console; the api base path is `/v1`.",
            }
        ]),
    );

    Value::Object(root)
}

fn error_response(description: &str) -> Value {
    json!({
        "description": description,
        "content": { "application/json": { "schema": error_schema() } },
    })
}

fn idempotency_parameter() -> Value {
    json!({
        "in": "header",
        "name": "idempotency-key",
        "required": false,
        "schema": { "type": "string", "maxLength": 128 },
        "description": "Within the retention window, a repeat of this key returns the stored response instead of re-running (and re-charging) the request.",
    })
}

fn job_id_parameter() -> Value {
    json!({
        "in": "path",
        "name": "job_id",
        "required": true,
        "schema": { "type": "string", "maxLength": 64 },
    })
}

/// Render the document the way it is served and the way it is checked in:
/// pretty-printed JSON with a trailing newline.
pub fn render(cfg: &ManagedApiCfg) -> String {
    let mut out =
        serde_json::to_string_pretty(&spec(cfg)).expect("the OpenAPI document serializes");
    out.push('\n');
    out
}

/// Headers the served document carries.
pub fn content_type() -> &'static str {
    "application/json"
}

/// The webhook signature header name, exposed for documentation tooling.
pub fn signature_header() -> &'static str {
    WEBHOOK_SIGNATURE_HEADER
}

/// The webhook timestamp header name, exposed for documentation tooling.
pub fn timestamp_header() -> &'static str {
    WEBHOOK_TIMESTAMP_HEADER
}

/// The bounds the deployment actually enforces, so a reader can tell the
/// documented limits from the ones in force.
pub fn config_summary(cfg: &ManagedApiCfg) -> Value {
    json!({
        "max_request_bytes": cfg.max_request_bytes,
        "max_redirects": cfg.max_redirects,
        "request_timeout_ms": cfg.request_timeout_ms,
        "max_url_len": cfg.max_url_len,
        "max_records": cfg.max_records,
        "jobs": {
            "enabled": cfg.jobs.enabled,
            "max_jobs": cfg.jobs.max_jobs,
            "ttl_secs": cfg.jobs.ttl_secs,
            "jobs_max_list": cfg.jobs.jobs_max_list,
        },
        "webhook": {
            "enabled": cfg.webhook.enabled,
            "max_attempts": cfg.webhook.max_attempts,
            "timeout_ms": cfg.webhook.timeout_ms,
            "allowed_hosts": cfg.webhook.allowed_hosts,
        },
        "pricing": {
            "currency": cfg.pricing.currency,
            "rate": cfg.pricing.rate,
            "base_units": cfg.pricing.base_units,
        },
    })
}

/// A short summary of the served contract, for the console.
pub fn summary(cfg: &ManagedApiCfg) -> Value {
    json!({
        "openapi": OPENAPI_VERSION,
        "request_bytes": cfg.max_request_bytes,
        "paths": ["/v1/scrape", "/v1/jobs", "/v1/jobs/{job_id}", "/v1/jobs/{job_id}/cancel", "/v1/usage", "/v1/openapi.json"],
        "auth": "bearer api key (hv1_...)",
        "signing": {
            "header": WEBHOOK_SIGNATURE_HEADER,
            "timestamp_header": WEBHOOK_TIMESTAMP_HEADER,
            "scheme": "HMAC-SHA256 over \"<timestamp>.<payload>\"",
        },
    })
}

/// Assertions the generated document must satisfy. Used by tests and by the
/// drift check, so a malformed spec fails loudly instead of being served.
pub fn validate(doc: &Value) -> Result<(), String> {
    let paths = doc
        .get("paths")
        .and_then(|p| p.as_object())
        .ok_or("the document has no paths object")?;
    if paths.is_empty() {
        return Err("the document declares no paths".to_string());
    }
    let required = [
        "/v1/scrape",
        "/v1/jobs",
        "/v1/jobs/{job_id}",
        "/v1/jobs/{job_id}/cancel",
        "/v1/usage",
        "/v1/openapi.json",
    ];
    for path in required {
        if !paths.contains_key(path) {
            return Err(format!("the document is missing {path}"));
        }
    }
    if doc.get("openapi").and_then(|v| v.as_str()) != Some(OPENAPI_VERSION) {
        return Err("the document does not declare openapi 3.1".to_string());
    }
    Ok(())
}

/// Convenience for config examples and the drift test.
pub fn config_example(cfg: &ManagedApiCfg) -> Value {
    json!({
        "max_request_bytes": cfg.max_request_bytes,
        "jobs": {
            "enabled": cfg.jobs.enabled,
            "workers": cfg.jobs.workers,
            "max_browser_ms": cfg.jobs.max_browser_ms,
            "max_records": cfg.jobs.max_records,
        },
        "webhook": {
            "enabled": cfg.webhook.enabled,
            "timeout_ms": cfg.webhook.timeout_ms,
            "max_attempts": cfg.webhook.max_attempts,
            "retry_schedule_ms": cfg.webhook.retry_schedule_ms,
        },
        "pricing": {
            "currency": cfg.pricing.currency,
            "rate": cfg.pricing.rate,
        },
    })
}
