//! Crawl planning — bounded, declarative URL orchestration.
//!
//! This module **plans**; it does not fetch. Ownership stays where the engine
//! already put it:
//!
//! | Concern | Owner (unchanged) |
//! |---|---|
//! | HTTP, redirects, retries, proxy failover | [`crate::fetcher`] |
//! | robots.txt, crawl-delay, politeness | [`crate::robots`] + [`crate::policy`] |
//! | rate limits, concurrency, budgets | [`crate::sched`] / [`crate::budget`] |
//! | resume from an interrupted run | [`crate::ckpt`] |
//! | extraction, transforms, schema, sinks | [`crate::extract`], [`crate::transform`], [`crate::schema`], [`crate::sink`] |
//!
//! The planner's job is to answer, deterministically: *what is allowed to be
//! requested next, and why not the rest*. It is a pure state machine over
//! (configuration, discovered candidates), which is why the whole thing is
//! testable without a network (`tests/crawl_discovery.rs`).
//!
//! Bounds are enforced here, not downstream: depth, page budget, per-page link
//! budget, content-type scope, host scope, robots, and include/exclude
//! patterns all gate admission. Nothing in this module relaxes an existing
//! politeness or reliability limit — it only refuses more.

pub mod discovery;

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::TargetCfg;
use crate::content::{classify_bounded, ContentInfo, ContentKind};
use crate::document::{canonical_url_key, DiscoverySource, Document, DocumentSet};
use crate::media::{self, MediaCollector, MediaPolicy};
use crate::robots::Robots;

/// Upper bound we accept for `max_depth`. A deeper crawl is almost always a
/// misconfiguration, and unbounded recursion is how polite crawlers become
/// incidents.
pub const MAX_DEPTH_CEILING: u32 = 20;
/// Upper bound for `max_pages` (per crawl plan).
pub const MAX_PAGES_CEILING: usize = 1_000_000;
/// Upper bound for per-page link expansion.
pub const MAX_LINKS_PER_PAGE_CEILING: usize = 10_000;

/// The content types a crawl is allowed to treat as record sources. Anything
/// outside this set is still *discoverable* (media/documents), but pages of
/// that type are not fetched for records.
pub const DEFAULT_CONTENT_TYPES: [ContentKind; 5] = [
    ContentKind::Html,
    ContentKind::Json,
    ContentKind::Xml,
    ContentKind::Csv,
    ContentKind::Text,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrawlError {
    NoStartUrls,
    InvalidStartUrl(String),
    DepthTooLarge { got: u32, max: u32 },
    PageBudgetTooLarge { got: usize, max: usize },
    LinkBudgetTooLarge { got: usize, max: usize },
    UnknownContentType(String),
}

impl std::fmt::Display for CrawlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoStartUrls => write!(f, "crawl needs at least one start URL"),
            Self::InvalidStartUrl(u) => write!(f, "start URL is not an http(s) URL: {u}"),
            Self::DepthTooLarge { got, max } => {
                write!(f, "max_depth {got} exceeds the ceiling {max}")
            }
            Self::PageBudgetTooLarge { got, max } => {
                write!(f, "max_pages {got} exceeds the ceiling {max}")
            }
            Self::LinkBudgetTooLarge { got, max } => {
                write!(f, "max_links_per_page {got} exceeds the ceiling {max}")
            }
            Self::UnknownContentType(t) => write!(f, "unknown content type `{t}`"),
        }
    }
}

impl std::error::Error for CrawlError {}

/// The declarative crawl contract. Serialized 1:1 as the JSON the Crawl Studio
/// console produces, and consumed by [`CrawlPlan::new`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlCfg {
    pub start_urls: Vec<String>,
    /// Substring include filter (empty = no filter). Same semantics as
    /// `TargetCfg::include`: a URL is kept if it contains any entry.
    #[serde(default)]
    pub include: Vec<String>,
    /// Substring exclude filter, evaluated before `include`.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// How many levels below a start URL may be crawled (0 = start URLs only).
    pub max_depth: u32,
    /// Total page budget for the plan.
    pub max_pages: usize,
    /// Cap on candidates taken from any single page.
    pub max_links_per_page: usize,
    /// Content types accepted as record sources (default: all textual kinds).
    #[serde(default = "default_content_types")]
    pub content_types: Vec<String>,
    #[serde(default)]
    pub discovery: discovery::DiscoveryCfg,
    /// Seed from `<origin>/sitemap.xml` (or `sitemap_url`) before crawling.
    #[serde(default)]
    pub sitemap: bool,
    #[serde(default)]
    pub sitemap_url: Option<String>,
    /// How deep `<sitemapindex>` recursion may go.
    #[serde(default = "default_one")]
    pub sitemap_max_depth: u32,
    /// Bodies larger than this are refused before decoding.
    #[serde(default = "default_max_response_bytes")]
    pub max_response_bytes: u64,
    /// Resource/media collection bounds.
    #[serde(default)]
    pub media: MediaPolicy,
    /// Cap on the document index the plan retains.
    #[serde(default = "default_documents_cap")]
    pub documents_cap: usize,
}

fn default_content_types() -> Vec<String> {
    DEFAULT_CONTENT_TYPES
        .iter()
        .map(|k| k.as_str().to_string())
        .collect()
}

fn default_one() -> u32 {
    1
}

fn default_max_response_bytes() -> u64 {
    crate::content::DEFAULT_MAX_BODY_BYTES
}

fn default_documents_cap() -> usize {
    10_000
}

impl Default for CrawlCfg {
    fn default() -> Self {
        Self {
            start_urls: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            max_depth: 1,
            max_pages: 100,
            max_links_per_page: 200,
            content_types: default_content_types(),
            discovery: discovery::DiscoveryCfg::default(),
            sitemap: false,
            sitemap_url: None,
            sitemap_max_depth: default_one(),
            max_response_bytes: default_max_response_bytes(),
            media: MediaPolicy::default(),
            documents_cap: default_documents_cap(),
        }
    }
}

/// One queued request the plan has admitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedUrl {
    pub url: String,
    pub depth: u32,
    pub source: DiscoverySource,
}

/// Why a candidate was refused. Every variant is counted in [`CrawlStats`] —
/// refuses are observable, not silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Skip {
    Robots,
    Excluded,
    NotIncluded,
    Depth,
    PageBudget,
    OffHost,
    Duplicate,
    AlreadyQueued,
    Unresolvable,
}

impl Skip {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Robots => "robots",
            Self::Excluded => "excluded",
            Self::NotIncluded => "not-included",
            Self::Depth => "depth",
            Self::PageBudget => "page-budget",
            Self::OffHost => "off-host",
            Self::Duplicate => "duplicate",
            Self::AlreadyQueued => "already-queued",
            Self::Unresolvable => "unresolvable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    Allow,
    Deny(Skip),
}

/// What one page added to the plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Expansion {
    pub candidates: usize,
    pub queued: usize,
    pub skipped: usize,
    pub documents: usize,
    pub media: usize,
    pub body_too_large: bool,
    pub kind: Option<ContentKind>,
}

/// Running counters. Reported by the console and the CLI; every number here is
/// measured, never estimated.
#[derive(Debug, Clone, Default, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlStats {
    pub queued: u64,
    pub visited: u64,
    pub discovered: u64,
    pub duplicate: u64,
    pub skipped_robots: u64,
    pub skipped_exclude: u64,
    pub skipped_include: u64,
    pub skipped_depth: u64,
    pub skipped_budget: u64,
    pub skipped_content_type: u64,
    pub skipped_offhost: u64,
    pub body_too_large: u64,
    pub sitemaps_seen: u64,
    pub sitemap_indexes: u64,
    pub documents: u64,
    pub media: u64,
    pub media_refused: u64,
}

/// The crawl plan: frontier + admission rules + bounded document/resource
/// indexes.
#[derive(Debug, Clone)]
pub struct CrawlPlan {
    cfg: CrawlCfg,
    frontier: VecDeque<PlannedUrl>,
    queued: HashSet<String>,
    visited: HashSet<String>,
    documents: DocumentSet,
    media: MediaCollector,
    stats: CrawlStats,
    robots: Option<Robots>,
    kinds: Vec<ContentKind>,
}

impl CrawlPlan {
    /// Validate the configuration and build an empty plan.
    pub fn new(cfg: CrawlCfg) -> Result<Self, CrawlError> {
        if cfg.start_urls.is_empty() {
            return Err(CrawlError::NoStartUrls);
        }
        for u in &cfg.start_urls {
            valid_http_url(u).ok_or_else(|| CrawlError::InvalidStartUrl(u.clone()))?;
        }
        if cfg.max_depth > MAX_DEPTH_CEILING {
            return Err(CrawlError::DepthTooLarge {
                got: cfg.max_depth,
                max: MAX_DEPTH_CEILING,
            });
        }
        if cfg.max_pages > MAX_PAGES_CEILING {
            return Err(CrawlError::PageBudgetTooLarge {
                got: cfg.max_pages,
                max: MAX_PAGES_CEILING,
            });
        }
        if cfg.max_links_per_page > MAX_LINKS_PER_PAGE_CEILING {
            return Err(CrawlError::LinkBudgetTooLarge {
                got: cfg.max_links_per_page,
                max: MAX_LINKS_PER_PAGE_CEILING,
            });
        }
        let mut kinds = Vec::new();
        for t in &cfg.content_types {
            let k =
                ContentKind::parse(t).ok_or_else(|| CrawlError::UnknownContentType(t.clone()))?;
            kinds.push(k);
        }
        if kinds.is_empty() {
            kinds = DEFAULT_CONTENT_TYPES.to_vec();
        }
        if let Some(u) = cfg.sitemap_url.as_deref() {
            valid_http_url(u).ok_or_else(|| CrawlError::InvalidStartUrl(u.to_string()))?;
        }
        let documents = DocumentSet::new(cfg.documents_cap.max(1));
        let media = MediaCollector::new(cfg.media.clone());
        Ok(Self {
            cfg,
            frontier: VecDeque::new(),
            queued: HashSet::new(),
            visited: HashSet::new(),
            documents,
            media,
            stats: CrawlStats::default(),
            robots: None,
            kinds,
        })
    }

    /// Adapt an existing [`TargetCfg`] into a crawl plan, reusing the target's
    /// depth/page/include/exclude/sitemap settings rather than a parallel set.
    pub fn from_target(t: &TargetCfg) -> Result<Self, CrawlError> {
        let depth = t.max_depth.min(MAX_DEPTH_CEILING as usize) as u32;
        let cfg = CrawlCfg {
            start_urls: vec![t.start_url.clone()],
            include: t.include.clone(),
            exclude: t.exclude.clone(),
            max_depth: depth,
            max_pages: t.max_pages.max(1),
            sitemap: t.sitemap,
            sitemap_url: t.sitemap_url.clone(),
            discovery: discovery::DiscoveryCfg {
                pagination_selector: t.link_selector.clone(),
                pagination: t.follow_links,
                documents: t.follow_links,
                media: false, // targets produce records, not media indexes
                ..discovery::DiscoveryCfg::default()
            },
            ..CrawlCfg::default()
        };
        Self::new(cfg)
    }

    pub fn cfg(&self) -> &CrawlCfg {
        &self.cfg
    }

    pub fn stats(&self) -> CrawlStats {
        self.stats
    }

    pub fn pending(&self) -> usize {
        self.frontier.len()
    }

    pub fn documents(&self) -> &DocumentSet {
        &self.documents
    }

    pub fn media(&self) -> &MediaCollector {
        &self.media
    }

    /// Install a parsed robots.txt. Until this is called, the plan refuses
    /// nothing on robots grounds but reports `robots_loaded == false` — it
    /// never *assumes* permission.
    pub fn set_robots(&mut self, robots: Robots) {
        self.robots = Some(robots);
    }

    pub fn set_robots_text(&mut self, text: &str, user_agent: &str) {
        self.robots = Some(Robots::parse(text, user_agent));
    }

    pub fn robots_loaded(&self) -> bool {
        self.robots.is_some()
    }

    /// Sitemap URLs advertised by the configured robots.txt (all of them).
    pub fn robots_sitemaps(&self, text: &str) -> Vec<String> {
        discovery::sitemaps_from_robots(text)
    }

    /// Seed the frontier from `start_urls` (depth 0). Returns how many were
    /// actually queued; refusals land in the stats.
    pub fn seed(&mut self) -> usize {
        let urls = self.cfg.start_urls.clone();
        let mut n = 0;
        for u in urls {
            if self.enqueue(u, 0, DiscoverySource::Start) {
                n += 1;
            }
        }
        n
    }

    /// Seed from a sitemap document: page URLs are queued at `depth`,
    /// child sitemaps are returned for the caller to fetch (bounded by
    /// `sitemap_max_depth`).
    pub fn seed_sitemap(&mut self, base: &str, xml: &str, depth: u32) -> SitemapExpansion {
        let Some(base_url) = valid_http_url(base) else {
            return SitemapExpansion::default();
        };
        let found = discovery::discover_sitemap(&base_url, xml, depth);
        self.stats.sitemaps_seen += 1;
        let mut out = SitemapExpansion {
            is_index: found.is_index,
            ..Default::default()
        };
        if found.is_index {
            self.stats.sitemap_indexes += 1;
            out.children = found.children.iter().map(|c| c.url.clone()).collect();
            out.children.truncate(self.cfg.max_links_per_page);
        } else {
            for u in &found.urls {
                if self.enqueue(u.url.clone(), depth, DiscoverySource::Sitemap) {
                    out.queued += 1;
                }
            }
        }
        out
    }

    /// Admission decision for one candidate URL at a depth. Pure: no mutation,
    /// so the console can preview the effective scope (Crawl Scope panel).
    pub fn admit(&self, url: &str, depth: u32) -> Admit {
        let Some(parsed) = valid_http_url(url) else {
            return Admit::Deny(Skip::Unresolvable);
        };
        if !self.host_allowed(&parsed) {
            return Admit::Deny(Skip::OffHost);
        }
        if let Some(r) = self.robots.as_ref() {
            let path = parsed.path();
            let path = if path.is_empty() { "/" } else { path };
            let query = parsed.query().unwrap_or("");
            let full = if query.is_empty() {
                path.to_string()
            } else {
                format!("{path}?{query}")
            };
            if !r.allows(&full) {
                return Admit::Deny(Skip::Robots);
            }
        }
        if discovery::matches_any(url, &self.cfg.exclude) {
            return Admit::Deny(Skip::Excluded);
        }
        if !self.cfg.include.is_empty() && !discovery::matches_any(url, &self.cfg.include) {
            return Admit::Deny(Skip::NotIncluded);
        }
        if depth > self.cfg.max_depth {
            return Admit::Deny(Skip::Depth);
        }
        if self.visited.len() + self.frontier.len() >= self.cfg.max_pages {
            return Admit::Deny(Skip::PageBudget);
        }
        let key = canonical_url_key(url);
        if self.visited.contains(&key) {
            return Admit::Deny(Skip::Duplicate);
        }
        if self.queued.contains(&key) {
            return Admit::Deny(Skip::AlreadyQueued);
        }
        Admit::Allow
    }

    /// Content-type scope: is a body of this kind wanted as record source?
    pub fn kind_allowed(&self, kind: ContentKind) -> bool {
        self.kinds.contains(&kind)
    }

    fn host_allowed(&self, url: &url::Url) -> bool {
        if !self.cfg.discovery.same_host_only {
            return true;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        let starts_with_host = self
            .cfg
            .start_urls
            .iter()
            .filter_map(|u| valid_http_url(u))
            .any(|p| p.host_str() == Some(host));
        if starts_with_host {
            return true;
        }
        // Sitemap-seeded URLs may live on a dedicated sitemap host; that host
        // is in scope, and nothing else is.
        match self.cfg.sitemap_url.as_deref() {
            Some(s) => valid_http_url(s)
                .and_then(|p| p.host_str().map(|h| h == host))
                .unwrap_or(false),
            None => false,
        }
    }

    fn enqueue(&mut self, url: String, depth: u32, source: DiscoverySource) -> bool {
        let key = canonical_url_key(&url);
        match self.admit(&url, depth) {
            Admit::Allow => {
                self.queued.insert(key);
                self.frontier.push_back(PlannedUrl { url, depth, source });
                self.stats.queued += 1;
                true
            }
            Admit::Deny(skip) => {
                self.count_skip(skip);
                false
            }
        }
    }

    fn count_skip(&mut self, skip: Skip) {
        match skip {
            Skip::Robots => self.stats.skipped_robots += 1,
            Skip::Excluded => self.stats.skipped_exclude += 1,
            Skip::NotIncluded => self.stats.skipped_include += 1,
            Skip::Depth => self.stats.skipped_depth += 1,
            Skip::PageBudget => self.stats.skipped_budget += 1,
            Skip::OffHost => self.stats.skipped_offhost += 1,
            Skip::Duplicate | Skip::AlreadyQueued => self.stats.duplicate += 1,
            Skip::Unresolvable => self.stats.duplicate += 1,
        }
    }

    /// Take the next `n` admitted requests (FIFO; BFS keeps the crawl shallow
    /// and predictable). Popping marks the URL visited.
    pub fn next(&mut self, n: usize) -> Vec<PlannedUrl> {
        let mut out = Vec::new();
        for _ in 0..n {
            let Some(p) = self.frontier.pop_front() else {
                break;
            };
            let key = canonical_url_key(&p.url);
            self.queued.remove(&key);
            self.visited.insert(key);
            self.stats.visited += 1;
            out.push(p);
        }
        out
    }

    /// Record a fetched page's document metadata and expand its candidates.
    /// Bodies over `max_response_bytes` are refused before decoding; pages
    /// whose content type is out of scope are recorded but not expanded.
    pub fn expand(
        &mut self,
        page: &str,
        body: &[u8],
        content_type: Option<&str>,
        depth: u32,
        fetched_ts: i64,
    ) -> Expansion {
        let mut out = Expansion::default();
        let info: ContentInfo =
            match classify_bounded(content_type, body, self.cfg.max_response_bytes) {
                Ok(i) => i,
                Err(_) => {
                    self.stats.body_too_large += 1;
                    out.body_too_large = true;
                    return out;
                }
            };
        out.kind = Some(info.kind);
        let doc = Document::observed(
            page,
            "crawl",
            &info,
            DiscoverySource::Link,
            depth,
            None,
            fetched_ts,
        )
        .with_sha256(Some(crate::document::sha256_hex(body)))
        .with_title(if info.kind == ContentKind::Html {
            crate::document::title_from_html(&String::from_utf8_lossy(body))
        } else {
            None
        });
        if self.documents.push(doc) == crate::document::PushOutcome::Added {
            out.documents += 1;
            self.stats.documents += 1;
        }

        if !self.kind_allowed(info.kind) {
            self.stats.skipped_content_type += 1;
            return out;
        }
        if !matches!(info.kind, ContentKind::Html | ContentKind::Xml) {
            return out; // JSON/CSV/text pages carry no links to discover
        }
        let base = match valid_http_url(page) {
            Some(u) => u,
            None => return out,
        };
        let text = crate::content::decode_text(&info, body);
        let next_depth = depth.saturating_add(1);
        let found = discovery::discover_html(&base, &text, &self.cfg.discovery, next_depth);

        // Order matters and is fixed: pagination, then links, then canonical,
        // then alternates — so a bounded budget spends itself on the pages an
        // operator actually meant to crawl.
        let mut candidates: Vec<&discovery::Discovered> = Vec::new();
        candidates.extend(found.pagination.iter());
        candidates.extend(found.links.iter());
        if let Some(c) = found.canonical.as_ref() {
            candidates.push(c);
        }
        candidates.extend(found.alternates.iter());
        out.candidates = candidates.len();

        for c in candidates {
            if out.queued >= self.cfg.max_links_per_page {
                break;
            }
            self.stats.discovered += 1;
            if self.enqueue(c.url.clone(), c.depth, c.source) {
                out.queued += 1;
            } else {
                out.skipped += 1;
            }
        }

        // Resources (documents + media) are recorded as metadata, never fetched
        // by the plan: size is unknown at discovery time, so the media policy's
        // `allow_unknown_size` gate decides whether they are indexed at all.
        let mut resources: Vec<&discovery::Discovered> = Vec::new();
        resources.extend(found.documents.iter());
        resources.extend(found.media.iter());
        for r in resources {
            let res = media::resource(r.url.clone(), None, None, None, page, r.source, r.depth);
            match self.media.push(res) {
                crate::media::PushOutcome::Added => {
                    out.media += 1;
                    self.stats.media += 1;
                }
                crate::media::PushOutcome::Duplicate => {}
                _ => self.stats.media_refused += 1,
            }
        }
        out
    }

    /// Snapshot for the console / CLI: configuration, counters, and the
    /// effective scope (what this plan would and would not request).
    pub fn to_json(&self) -> Value {
        json!({
            "config": {
                "start_urls": self.cfg.start_urls,
                "include": self.cfg.include,
                "exclude": self.cfg.exclude,
                "max_depth": self.cfg.max_depth,
                "max_pages": self.cfg.max_pages,
                "max_links_per_page": self.cfg.max_links_per_page,
                "content_types": self.cfg.content_types,
                "sitemap": self.cfg.sitemap,
                "sitemap_url": self.cfg.sitemap_url,
                "max_response_bytes": self.cfg.max_response_bytes,
                "documents_cap": self.cfg.documents_cap,
            },
            "pending": self.pending(),
            "visited": self.visited.len(),
            "stats": self.stats,
            "documents": self.documents.len(),
            "media": self.media.len(),
            "robots_loaded": self.robots_loaded(),
        })
    }
}

/// Result of feeding one sitemap document to [`CrawlPlan::seed_sitemap`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SitemapExpansion {
    pub queued: usize,
    pub children: Vec<String>,
    pub is_index: bool,
}

/// `Some` only for absolute http(s) URLs with a host — everything else is
/// refused rather than silently resolved against a guessed base.
fn valid_http_url(u: &str) -> Option<url::Url> {
    let parsed = url::Url::parse(u).ok()?;
    if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() {
        Some(parsed)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(url: &str) -> CrawlCfg {
        CrawlCfg {
            start_urls: vec![url.to_string()],
            ..CrawlCfg::default()
        }
    }

    #[test]
    fn refuses_configurations_that_cannot_be_bounded() {
        assert!(CrawlPlan::new(CrawlCfg::default()).is_err());
        assert!(CrawlPlan::new(cfg("ftp://x.test")).is_err());
        assert!(CrawlPlan::new(CrawlCfg {
            max_depth: 99,
            ..cfg("https://x.test")
        })
        .is_err());
        assert!(CrawlPlan::new(CrawlCfg {
            max_pages: 10_000_000,
            ..cfg("https://x.test")
        })
        .is_err());
        assert!(CrawlPlan::new(CrawlCfg {
            content_types: vec!["nope".to_string()],
            ..cfg("https://x.test")
        })
        .is_err());
    }

    #[test]
    fn depth_and_budget_gate_admission() {
        let mut p = CrawlPlan::new(CrawlCfg {
            max_depth: 1,
            max_pages: 2,
            ..cfg("https://x.test/")
        })
        .unwrap();
        assert_eq!(p.seed(), 1);
        assert_eq!(p.admit("https://x.test/a", 1), Admit::Allow);
        assert_eq!(p.admit("https://x.test/a", 2), Admit::Deny(Skip::Depth));
        let body = b"<html><body><a href=\"/a\">a</a><a href=\"/b\">b</a><a href=\"/c\">c</a></body></html>";
        let e = p.expand("https://x.test/", body, Some("text/html"), 0, 0);
        assert_eq!(e.candidates, 3);
        // Budget counts pending + visited: the start URL still occupies one slot.
        assert_eq!(e.queued, 1);
        assert_eq!(p.stats().skipped_budget, 2);
    }

    #[test]
    fn include_exclude_and_host_scope_apply() {
        let mut c = cfg("https://x.test/");
        c.include = vec!["/keep".to_string()];
        c.exclude = vec!["/keep/but-not".to_string()];
        let p = CrawlPlan::new(c).unwrap();
        assert_eq!(p.admit("https://x.test/keep/a", 0), Admit::Allow);
        assert_eq!(
            p.admit("https://x.test/keep/but-not", 0),
            Admit::Deny(Skip::Excluded)
        );
        assert_eq!(
            p.admit("https://x.test/other", 0),
            Admit::Deny(Skip::NotIncluded)
        );
        assert_eq!(
            p.admit("https://other.test/keep/a", 0),
            Admit::Deny(Skip::OffHost)
        );
    }

    #[test]
    fn robots_is_authoritative_once_loaded() {
        let mut p = CrawlPlan::new(cfg("https://x.test/")).unwrap();
        assert_eq!(
            p.admit("https://x.test/private", 0),
            Admit::Allow,
            "no robots => not assumed-allowed, but nothing to enforce yet"
        );
        p.set_robots_text("User-agent: *\nDisallow: /private\n", "harvest");
        assert!(p.robots_loaded());
        assert_eq!(
            p.admit("https://x.test/private", 0),
            Admit::Deny(Skip::Robots)
        );
        assert_eq!(p.admit("https://x.test/public", 0), Admit::Allow);
    }

    #[test]
    fn out_of_scope_content_types_are_recorded_but_not_expanded() {
        let mut p = CrawlPlan::new(cfg("https://x.test/")).unwrap();
        let body = b"<html><body><a href=\"/a\">a</a></body></html>";
        let e = p.expand("https://x.test/p", body, Some("image/png"), 0, 0);
        assert_eq!(e.kind, Some(ContentKind::Binary));
        assert_eq!(e.queued, 0);
        assert_eq!(p.stats().skipped_content_type, 1);
        assert_eq!(p.documents().len(), 1, "still recorded as a document");
    }

    #[test]
    fn oversized_bodies_are_refused_before_decoding() {
        let mut c = cfg("https://x.test/");
        c.max_response_bytes = 10;
        let mut p = CrawlPlan::new(c).unwrap();
        let e = p.expand("https://x.test/p", &[b'a'; 64], Some("text/html"), 0, 0);
        assert!(e.body_too_large);
        assert_eq!(p.stats().body_too_large, 1);
    }

    #[test]
    fn sitemap_seeding_and_index_recursion() {
        let mut p = CrawlPlan::new(cfg("https://x.test/")).unwrap();
        let index =
            "<sitemapindex><sitemap><loc>https://x.test/s1.xml</loc></sitemap></sitemapindex>";
        let out = p.seed_sitemap("https://x.test/sitemap.xml", index, 0);
        assert!(out.is_index);
        assert_eq!(out.children, vec!["https://x.test/s1.xml".to_string()]);
        assert_eq!(out.queued, 0);
        let flat =
            "<urlset><url><loc>https://x.test/p1</loc></url><url><loc>/p2</loc></url></urlset>";
        let out2 = p.seed_sitemap("https://x.test/sitemap.xml", flat, 0);
        assert!(!out2.is_index);
        assert_eq!(out2.queued, 2);
        assert_eq!(p.pending(), 2);
    }

    #[test]
    fn fifo_order_and_visited_tracking() {
        let mut p = CrawlPlan::new(CrawlCfg {
            max_pages: 10,
            ..cfg("https://x.test/")
        })
        .unwrap();
        p.seed();
        let body = b"<html><body><a href=\"/a\">a</a><a href=\"/b\">b</a></body></html>";
        p.expand("https://x.test/", body, Some("text/html"), 0, 0);
        let batch = p.next(10);
        assert_eq!(
            batch.iter().map(|p| p.url.as_str()).collect::<Vec<_>>(),
            vec!["https://x.test/", "https://x.test/a", "https://x.test/b"]
        );
        assert_eq!(p.pending(), 0);
        assert_eq!(p.stats().visited, 3, "start URL + the two discovered pages");
        assert_eq!(p.admit("https://x.test/a", 1), Admit::Deny(Skip::Duplicate));
    }
}
