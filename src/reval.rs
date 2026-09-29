//! Incremental revalidation: conditional GET + a cached link graph.
//!
//! On a 200 we remember the response's `ETag` / `Last-Modified` **and the
//! links we discovered in the body**. Next run, each known URL is requested
//! with `If-None-Match` / `If-Modified-Since`; a 304 costs no body transfer,
//! no parse, no new record — but discovery *replays from the cached link
//! list*, so the crawl keeps advancing (new pages linked from an unchanged
//! index are still found and fetched). That last part is what makes
//! revalidation an incremental crawler instead of a records-suppressor.
//!
//! Politeness is never skipped: every revalidation is still a request,
//! paced per-host and robots-gated like any other — we just stop hauling
//! bytes the site already confirmed unchanged.
//!
//! The store is a single JSONL file (header line + one line per URL),
//! written atomically (temp + rename). Entries that gain no validators and
//! no link set are dropped on save — a conditional request they'd enable
//! could never be answered with 304, so keeping them would only grow the
//! file and burn pointless revalidations that fall through to full fetches.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Cap the cached link set per document: nav-heavy pages (thousands of hrefs)
/// must not turn the store into a memory hog. Above the cap, a 304 replays
/// only the first `LINK_CAP` links — acceptable, documented.
pub const LINK_CAP: usize = 500;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UrlState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lm: Option<String>,
    /// hrefs (already absolutized & filtered) discovered on last 200.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// unix secs of the last confirmed-fresh event (200 or 304).
    pub ts: i64,
}

/// What to attach to the next request for this URL, if anything.
#[derive(Debug, PartialEq)]
pub enum Condition {
    None,
    Headers(Vec<(String, String)>),
    /// Entry exists but expired by `revalidate_ttl_secs` — fetch for real.
    Expired,
}

pub struct Store {
    path: PathBuf,
    ttl_secs: u64,
    map: Mutex<HashMap<String, UrlState>>,
    since_save: AtomicUsize,
}

impl Store {
    pub fn load(path: PathBuf, ttl_secs: u64) -> Result<Self> {
        let mut map = HashMap::new();
        if path.exists() {
            let txt = std::fs::read_to_string(&path)
                .with_context(|| format!("read validators {}", path.display()))?;
            for (i, line) in txt.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if i == 0 && line.starts_with("{\"v\":") {
                    continue; // header
                }
                match serde_json::from_str::<(String, UrlState)>(line) {
                    Ok((k, v)) => {
                        map.insert(k, v);
                    }
                    Err(e) => {
                        tracing::warn!(%e, line = i, "ignoring corrupt validators line");
                    }
                }
            }
        }
        Ok(Self {
            path,
            ttl_secs,
            map: Mutex::new(map),
            since_save: AtomicUsize::new(0),
        })
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Decide the conditional headers for `key`.
    pub fn condition_for(&self, key: &str) -> Condition {
        let g = self.map.lock().unwrap();
        let Some(e) = g.get(key) else {
            return Condition::None;
        };
        if self.ttl_secs > 0 && (Self::now() - e.ts) as u64 > self.ttl_secs {
            return Condition::Expired;
        }
        let mut h = Vec::new();
        if let Some(t) = &e.etag {
            h.push(("if-none-match".to_string(), t.clone()));
        }
        if let Some(t) = &e.lm {
            h.push(("if-modified-since".to_string(), t.clone()));
        }
        if h.is_empty() {
            // Nothing to validate with — but a cached link set still makes a
            // plain fetch wasteful only in body terms; the server would just
            // 200. Skip conditionals entirely then.
            return Condition::None;
        }
        Condition::Headers(h)
    }

    /// 304 received: refresh freshness. If the entry can't ever enable
    /// replay-worthy behavior (no validators left to keep + nothing cached),
    /// callers drop it instead (see engine).
    pub fn touch(&self, key: &str) {
        if let Some(e) = self.map.lock().unwrap().get_mut(key) {
            e.ts = Self::now();
        }
    }

    /// 200 received: record validators + the (capped) link set.
    pub fn note_200(
        &self,
        key: &str,
        etag: Option<String>,
        lm: Option<String>,
        links: Vec<String>,
    ) {
        let links: Vec<String> = links.into_iter().take(LINK_CAP).collect();
        let mut g = self.map.lock().unwrap();
        let e = g.entry(key.to_string()).or_default();
        e.etag = etag;
        e.lm = lm;
        e.links = links;
        e.ts = Self::now();
        let n = self.since_save.fetch_add(1, Ordering::Relaxed) + 1;
        if n >= 500 {
            self.since_save.store(0, Ordering::Relaxed);
            if let Err(e) = self.save_locked_view() {
                tracing::warn!(%e, "validators mid-run save failed");
            }
        }
    }

    pub fn snapshot(&self, key: &str) -> Option<UrlState> {
        self.map.lock().unwrap().get(key).cloned()
    }

    /// Drop an entry (used when a 304 reveals it holds nothing useful).
    pub fn drop_key(&self, key: &str) {
        self.map.lock().unwrap().remove(key);
    }

    /// Atomic write; only entries that earn their keep are persisted.
    pub fn save(&self) -> Result<()> {
        self.save_locked_view()
    }

    fn save_locked_view(&self) -> Result<()> {
        let g = self.map.lock().unwrap();
        let mut out = String::with_capacity(64 + g.len() * 96);
        out.push_str("{\"v\":1}\n");
        for (k, e) in g.iter() {
            if e.etag.is_none() && e.lm.is_none() && e.links.is_empty() {
                continue; // useless for future revalidation
            }
            let line = serde_json::to_string(&(k, e)).context("serialize validator")?;
            out.push_str(&line);
            out.push('\n');
        }
        drop(g);
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p).ok();
        }
        let tmp = self
            .path
            .with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, out).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("rename -> {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
