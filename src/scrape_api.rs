//! The managed customer API (`/v1/*`).
//!
//! A second, API-key-authenticated surface on the same port as the operator
//! console. Tenants and keys live in [`crate::tenancy`], every request is
//! bounded by the key's plan, metered through the append-only
//! [`crate::usage::ledger`], and returned through a single canonical
//! response shape.
//!
//! The console router is asynchronous and awaits [`handlers::dispatch_async`]
//! directly. [`App`] still owns a dedicated Tokio runtime: background work
//! (job execution, webhook delivery) is scheduled on it with
//! [`App::rt`], which is a `Handle`, never a nested `block_on`.
//!
//! See `docs/modules/scrape_api.md`.

pub mod handlers;
pub mod idempotency;
pub mod jobs;
pub mod models;
pub mod openapi;
pub mod pricing;
pub mod service;
pub mod validate;
pub mod webhook;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::config::{Config, ManagedApiCfg};
use crate::metrics::Metrics;
use crate::ratelimit::RateLimiter;
use crate::tenancy::TenancyStore;
use crate::usage::ledger::UsageLedger;

use models::ChainHead;

/// Current wall-clock time in milliseconds since the Unix epoch.
pub fn time_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Every component the managed API needs, grouped so that one config produces
/// one `App` and `admin.rs` does not have to thread nine values through.
pub struct AppInputs {
    pub config: Arc<Config>,
    pub managed: ManagedApiCfg,
    pub store: Arc<TenancyStore>,
    pub limiter: Arc<RateLimiter>,
    pub jobs: Arc<jobs::JobRegistry>,
    pub idempotency: Arc<idempotency::IdempotencyStore>,
    pub ledger: Arc<UsageLedger>,
    pub metrics: Arc<Metrics>,
    pub webhooks: webhook::WebhookSender,
}

impl AppInputs {
    /// Build every component from an already-validated config. Each
    /// subsystem owns its own sub-directory under the managed API root, so no
    /// file mixes two responsibilities.
    pub fn from_config(
        config: Arc<Config>,
        managed: ManagedApiCfg,
        metrics: Arc<Metrics>,
    ) -> Result<Self> {
        let root = managed
            .data_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("state/managed-api"));
        let jobs = Arc::new(
            jobs::JobRegistry::open(&managed, root.join("jobs"))
                .context("cannot open the job registry")?,
        );
        let idempotency = Arc::new(
            idempotency::IdempotencyStore::open(&managed, root.join("idempotency"))
                .context("cannot open the idempotency store")?,
        );
        let ledger_dir = managed.ledger_dir();
        let webhooks = webhook::WebhookSender::new(&managed, root.join("webhooks"), &metrics)
            .context("cannot open the webhook sender")?;
        let store = TenancyStore::open(root.join("tenants"))
            .map_err(|e| anyhow::anyhow!("cannot open the tenancy store: {e}"))?;
        Ok(Self {
            config,
            managed,
            store,
            limiter: Arc::new(RateLimiter::new()),
            jobs,
            idempotency,
            ledger: UsageLedger::open(ledger_dir).context("cannot open the usage ledger")?,
            metrics,
            webhooks,
        })
    }
}

/// The whole managed API, cheap to clone (everything inside is an `Arc`).
#[derive(Clone)]
pub struct App {
    inner: Arc<AppInner>,
}

struct AppInner {
    config: Arc<Config>,
    managed: ManagedApiCfg,
    store: Arc<TenancyStore>,
    limiter: Arc<RateLimiter>,
    jobs: Arc<jobs::JobRegistry>,
    idempotency: Arc<idempotency::IdempotencyStore>,
    ledger: Arc<UsageLedger>,
    metrics: Arc<Metrics>,
    service: service::ScrapeService,
    pricing: pricing::Pricing,
    webhooks: webhook::WebhookSender,
    rt: tokio::runtime::Runtime,
}

impl App {
    /// Build the API from validated inputs. The runtime is created here,
    /// once, and never from inside an async context of itself.
    pub fn new(inputs: AppInputs) -> Result<Self> {
        let AppInputs {
            config,
            managed,
            store,
            limiter,
            jobs,
            idempotency,
            ledger,
            metrics,
            webhooks,
        } = inputs;
        if !managed.enabled {
            anyhow::bail!("the managed api is disabled in this configuration");
        }
        let service =
            service::ScrapeService::new(config.clone(), managed.clone(), metrics.clone())?;
        let pricing = pricing::Pricing::from_cfg(&managed);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .thread_name("managed-api")
            .build()
            .context("cannot build the managed API runtime")?;
        Ok(Self {
            inner: Arc::new(AppInner {
                config,
                managed,
                store,
                limiter,
                jobs,
                idempotency,
                ledger,
                metrics,
                service,
                pricing,
                webhooks,
                rt,
            }),
        })
    }

    pub fn config(&self) -> &Arc<Config> {
        &self.inner.config
    }

    pub fn managed(&self) -> &ManagedApiCfg {
        &self.inner.managed
    }

    pub fn store(&self) -> &Arc<TenancyStore> {
        &self.inner.store
    }

    pub fn limiter(&self) -> &Arc<RateLimiter> {
        &self.inner.limiter
    }

    pub fn jobs(&self) -> &Arc<jobs::JobRegistry> {
        &self.inner.jobs
    }

    pub fn idempotency(&self) -> &Arc<idempotency::IdempotencyStore> {
        &self.inner.idempotency
    }

    pub fn ledger(&self) -> &Arc<UsageLedger> {
        &self.inner.ledger
    }

    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.inner.metrics
    }

    pub fn service(&self) -> &service::ScrapeService {
        &self.inner.service
    }

    pub fn pricing(&self) -> pricing::Pricing {
        self.inner.pricing.clone()
    }

    pub fn webhooks(&self) -> &webhook::WebhookSender {
        &self.inner.webhooks
    }

    pub fn rt(&self) -> &tokio::runtime::Runtime {
        &self.inner.rt
    }

    /// The head of the usage ledger, echoed on every response so a caller
    /// can see that the request was metered.
    pub fn ledger_head(&self) -> ChainHead {
        ChainHead {
            seq: self.inner.ledger.seq(),
            hash: self.inner.ledger.last_hash().clone(),
        }
    }
}

/// Re-exported so the handlers and tests share one vocabulary.
pub use models::RESPONSE_SCHEMA;
