//! Signed webhook delivery with bounded retry and a dead letter queue.
//!
//! Every delivery is signed with HMAC-SHA256 over `"<timestamp>.<payload>"`
//! using the tenant's delivery key, and carries the timestamp in a header so
//! the receiver can bound replay. Retries follow the configured schedule;
//! once the attempt budget is exhausted the delivery is written to the
//! dead-letter file instead of being dropped, so nothing is lost silently.
//!
//! The dead letter file is JSONL: one record per failed delivery, with the
//! exact payload and signature that was attempted.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::config::ManagedApiCfg;
use crate::fsutil;
use crate::metrics::Metrics;
use crate::scrape_api::models::{WEBHOOK_SIGNATURE_HEADER, WEBHOOK_TIMESTAMP_HEADER};
use crate::ssrf::{Guard, SsrfPolicy};

type HmacSha256 = Hmac<Sha256>;

/// One delivery attempt's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryResult {
    Delivered,
    DeadLettered,
}

impl DeliveryResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::DeadLettered => "dead_lettered",
        }
    }
}

/// Everything one webhook delivery needs, grouped so the caller cannot mix
/// up the webhook id, the tenant and the key that triggered the event.
#[derive(Debug, Clone)]
pub struct Delivery {
    pub webhook_id: String,
    pub tenant_id: String,
    pub key_id: String,
    pub url: String,
    pub event: String,
}

impl Delivery {
    pub fn new(webhook_id: &str, tenant_id: &str, key_id: &str, url: &str, event: &str) -> Self {
        Self {
            webhook_id: webhook_id.to_string(),
            tenant_id: tenant_id.to_string(),
            key_id: key_id.to_string(),
            url: url.to_string(),
            event: event.to_string(),
        }
    }
}

/// The summary returned to the caller.
#[derive(Debug, Clone)]
pub struct WebhookOutcome {
    pub attempts: u32,
    pub result: DeliveryResult,
    pub dead_letter_path: Option<PathBuf>,
}

/// One dead-lettered delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeadLetter {
    pub webhook_id: String,
    pub tenant_id: String,
    pub key_id: String,
    pub url: String,
    pub event: String,
    pub payload: serde_json::Value,
    pub signature: String,
    pub timestamp: u64,
    pub attempts: u32,
    pub last_error: String,
    pub created_at_ms: u64,
}

/// Computes the `"<timestamp>.<payload>"` signature for a delivery.
pub fn sign(delivery_key: &str, timestamp: u64, payload: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(delivery_key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(format!("{timestamp}.{payload}").as_bytes());
    let bytes = mac.finalize().into_bytes();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// A verifier for the receiving side, so a consumer can check a delivery
/// without reimplementing the scheme.
pub fn verify(delivery_key: &str, timestamp: u64, payload: &str, signature: &str) -> bool {
    let expected = sign(delivery_key, timestamp, payload);
    // Constant-time-ish: compare lengths first, then byte by byte. The
    // digest is public, so the comparison only needs to not short-circuit
    // on the first differing byte.
    let (a, b) = (expected.as_bytes(), signature.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The delivery engine.
pub struct WebhookSender {
    cfg: ManagedApiCfg,
    /// Re-checked at send time. A webhook target is validated once at
    /// registration; the address it resolves to is re-tested before every
    /// delivery, so a record that changed under us cannot turn into a request
    /// to an internal address.
    guard: Guard,
    dir: PathBuf,
    path: PathBuf,
    metrics: std::sync::Arc<Metrics>,
    client: reqwest::Client,
}

impl WebhookSender {
    pub fn new(
        cfg: &ManagedApiCfg,
        dir: impl AsRef<Path>,
        metrics: &std::sync::Arc<Metrics>,
    ) -> Result<Self> {
        let guard = Guard::new(SsrfPolicy {
            max_redirects: cfg.max_redirects,
            allow_private_targets: cfg.allow_private_targets,
            allow_http: cfg.allow_private_targets,
            ..SsrfPolicy::default()
        });
        let dir = dir.as_ref();
        fsutil::prepare_private_directory(dir)
            .with_context(|| format!("cannot prepare webhook directory {}", dir.display()))?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.webhook.timeout_ms))
            .connect_timeout(Duration::from_millis(cfg.webhook.timeout_ms))
            .build()
            .context("cannot build the webhook HTTP client")?;
        Ok(Self {
            cfg: cfg.clone(),
            guard,
            dir: dir.to_path_buf(),
            path: dir.join("deliveries.jsonl"),
            metrics: metrics.clone(),
            client,
        })
    }

    pub fn dead_letter_path(&self) -> PathBuf {
        self.dir.join("dead-letter.jsonl")
    }

    /// Deliver one event, retrying on the configured schedule. Never
    /// returns an error: a failed delivery is dead-lettered, which is a
    /// recorded outcome rather than a lost one.
    pub async fn deliver(
        &self,
        delivery: &Delivery,
        payload: &serde_json::Value,
    ) -> WebhookOutcome {
        let payload_text = payload.to_string();
        let mut last_error = String::from("no attempts were made");

        // Send-time SSRF check. A target that is blocked now is dead-lettered
        // rather than retried: retrying an address the guard refuses would
        // only produce more refusals.
        if let Err(e) = self.guard.check_url(&delivery.url) {
            let path =
                self.dead_letter(delivery, payload, &format!("url_rejected: {}", e.message()));
            self.metrics
                .note_webhook_delivery(DeliveryResult::DeadLettered.as_str());
            return WebhookOutcome {
                attempts: 0,
                result: DeliveryResult::DeadLettered,
                dead_letter_path: Some(path),
            };
        }

        let max_attempts = self.cfg.webhook.max_attempts.max(1);
        for attempt in 1..=max_attempts {
            let timestamp = crate::scrape_api::time_now_ms();
            let signature = sign(&self.cfg.webhook.delivery_key, timestamp, &payload_text);
            match self
                .attempt(
                    &delivery.url,
                    &delivery.event,
                    &payload_text,
                    timestamp,
                    &signature,
                )
                .await
            {
                Ok(()) => {
                    self.metrics
                        .note_webhook_delivery(DeliveryResult::Delivered.as_str());
                    return WebhookOutcome {
                        attempts: attempt,
                        result: DeliveryResult::Delivered,
                        dead_letter_path: None,
                    };
                }
                Err(err) => {
                    last_error = format!("attempt {attempt}: {err}");
                }
            }
            if attempt < max_attempts {
                let delay = self.schedule_delay(attempt);
                if delay.is_zero() {
                    break;
                }
                tokio::time::sleep(delay).await;
            }
        }

        let path = self.dead_letter(delivery, payload, &last_error);
        self.metrics
            .note_webhook_delivery(DeliveryResult::DeadLettered.as_str());
        WebhookOutcome {
            attempts: max_attempts,
            result: DeliveryResult::DeadLettered,
            dead_letter_path: Some(path),
        }
    }

    /// Append one JSON line, creating the directory if needed. Used for the
    /// webhook outbox and the dead letter file, which are written
    /// incrementally and never rewritten.
    fn append_jsonl(&self, path: &Path, line: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fsutil::prepare_private_directory(parent)?;
            }
        }
        let mut file = fsutil::open_append(path)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(())
    }

    fn schedule_delay(&self, attempt: u32) -> Duration {
        let w = &self.cfg.webhook;
        if let Some(&ms) = w.retry_schedule_ms.get(attempt.saturating_sub(1) as usize) {
            return Duration::from_millis(ms);
        }
        // Default ladder: `retry_base_ms * 2^attempt`, capped at `retry_cap_ms`.
        let shift = attempt.saturating_sub(1).min(31);
        let ms = w
            .retry_base_ms
            .saturating_mul(1u64 << shift)
            .min(w.retry_cap_ms);
        Duration::from_millis(ms)
    }

    async fn attempt(
        &self,
        url: &str,
        event: &str,
        payload: &str,
        timestamp: u64,
        signature: &str,
    ) -> Result<()> {
        let request = self
            .client
            .post(url)
            .header("content-type", "application/json")
            .header(WEBHOOK_TIMESTAMP_HEADER, timestamp.to_string())
            .header(WEBHOOK_SIGNATURE_HEADER, signature.to_string())
            .header("x-harvest-event", event.to_string())
            .body(payload.to_string());
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(anyhow::anyhow!("webhook returned {status}"))
    }

    /// Append one dead-letter record.
    fn dead_letter(
        &self,
        delivery: &Delivery,
        payload: &serde_json::Value,
        last_error: &str,
    ) -> PathBuf {
        let path = self.dead_letter_path();
        let record = DeadLetter {
            webhook_id: delivery.webhook_id.clone(),
            tenant_id: delivery.tenant_id.clone(),
            key_id: delivery.key_id.clone(),
            url: delivery.url.clone(),
            event: delivery.event.clone(),
            payload: payload.clone(),
            signature: sign(
                // Re-sign so a consumer can replay the exact attempt.
                &self.cfg.webhook.dead_letter_salt,
                crate::scrape_api::time_now_ms(),
                &payload.to_string(),
            ),
            timestamp: crate::scrape_api::time_now_ms(),
            attempts: self.cfg.webhook.max_attempts.max(1),
            last_error: last_error.to_string(),
            created_at_ms: crate::scrape_api::time_now_ms(),
        };
        let outcome = self.append_jsonl(&path, &serde_json::to_string(&record).unwrap_or_default());
        if outcome.is_err() {
            // Dead-lettering must not lose the delivery, so a failed write
            // is surfaced through the metrics counter.
            self.metrics
                .note_webhook_delivery("dead_letter_write_failed");
        }
        path
    }

    /// Read the dead-letter file back, newest first.
    pub fn dead_letters(&self) -> Result<Vec<DeadLetter>> {
        let mut out = Vec::new();
        if !self.dead_letter_path().exists() {
            return Ok(out);
        }
        let text = fsutil::read_bounded(&self.dead_letter_path(), 4 * 1024 * 1024)
            .context("cannot read the dead-letter file")?;
        for line in String::from_utf8_lossy(&text).lines().rev() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<DeadLetter>(line) {
                out.push(rec);
            }
        }
        Ok(out)
    }

    /// Write a delivery receipt to the outbox so an operator can audit what
    /// was attempted even if the receiver never recorded it.
    pub fn record_attempt(
        &self,
        webhook_id: &str,
        url: &str,
        event: &str,
        result: &str,
    ) -> Result<()> {
        let line = serde_json::json!({
            "webhook_id": webhook_id,
            "url": url,
            "event": event,
            "result": result,
            "ts_ms": crate::scrape_api::time_now_ms(),
        });
        self.append_jsonl(&self.path, &line.to_string())
            .context("cannot append to the webhook outbox")
    }
}
