//! harvest — a compliance-first data-collection engine.
//!
//! Design pillars:
//! * **Politeness by construction**: robots.txt enforcement, per-domain token
//!   buckets, Retry-After respect, and a blocking circuit breaker. When a site
//!   says "no", the engine backs off and reports — it does not attempt to
//!   disguise the request.
//! * **Reliability**: upstream proxy pools are managed for *quality* (EWMA
//!   latency/success scoring, failover), not for evasion.
//! * **Typed extraction**: CSS selector rules -> JSON records, content-hash
//!   dedup, pluggable sinks (JSONL, CSV, SQLite).
//! * **Observable**: Prometheus text exposition + JSON stats endpoint, a
//!   structured event bus (JSONL log + `/events` tail) and a runtime
//!   snapshot (`/runtime`) over the engine layer below.
//!
//! The engine layer — job lifecycle, scheduling, supervision, transport
//! resilience, accounting and events — lives in dedicated modules:
//! * [`job`] — the unit of work: identity, state machine, deadlines, retries.
//! * [`sched`] — the queue workers pull from: FIFO/priority/fair, bounded.
//! * [`supervisor`] — worker registry, restart policy, drain, bounded tasks.
//! * [`exec`] — request plan/context/outcome envelope with correlation ids.
//! * [`retry`] — categorized retry policy: the delay curve, budgets, history.
//! * [`breaker`] — per-host transport circuit (closed/open/half-open).
//! * [`events`] — bounded ring event bus (async-safe, rejoin-aware).
//! * [`accounting`] — run-scoped resource counters and failure history.
//! * [`runtime`] — everything above, snapshotted as one JSON view.
//! * [`api`] — typed models + bounded queries shared by the admin API and CLI.
//! * [`ops`] — operator-plane state: bounded ops/audit rings and diagnostics.

pub mod accounting;

pub mod admin;
/// Optional, budgeted AI assistance (off by default; never authoritative).
pub mod ai;
pub mod alerts;
/// Typed API domain models, query bounds, error mapping, admin client.
pub mod api;
pub mod api_models;
pub mod artifact;
pub mod audit;
pub mod breaker;
/// Bounded resource budgets (acquire/release protocol shared by in-flight
/// holds and consumption caps).
pub mod budget;
pub mod cache;
pub mod ckpt;
pub mod config;
/// Bounded crawl planning: discovery, admission, and document/resource indexes.
pub mod content;
pub mod control;
pub mod crawl;
pub mod document;
pub mod media;

pub mod ai_job;
#[cfg(feature = "redis")]
pub mod dist;
pub mod download;
pub mod engine;
pub mod events;
pub mod exec;
pub mod execution;
pub mod export;
pub mod export_job;
pub mod extract;
pub mod fetcher;
pub mod health;
/// Typed identifiers and shared value semantics (ids, timestamps, partitions).
pub mod ids;
pub mod job;
pub mod metrics;
/// Config schema versioning and the migration table (see module docs).
pub mod migrate;
/// Operator-plane state: bounded ops/audit rings, redacted run views.
pub mod ops;
#[cfg(feature = "postgres")]
pub mod pgsink;
pub mod policy;
pub mod policy_engine;
pub mod proxy;
pub mod repository;
pub mod retry;
pub mod reval;
pub mod robots;
pub mod runner;
pub mod runtime;
pub mod runtime_metrics;
pub mod sched;
pub mod schema;
pub mod search;
pub mod sign;
pub mod sink;
pub mod sitemap;
pub mod stream;
pub mod supervisor;
/// Dev-only test harness (unit tests + `devtools` feature; never in normal builds).
#[cfg(any(test, feature = "devtools"))]
pub mod testutil;
pub mod transform;

pub use config::Config;
pub use engine::{run, Summary};

#[cfg(test)]
mod tests;
