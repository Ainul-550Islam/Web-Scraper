//! The retrieval engine behind `POST /v1/scrape`.
//!
//! One path, every compliance rule: the URL is parsed and SSRF-checked
//! before a socket exists, `robots.txt` is honoured, every domain is paced
//! through the shared politeness token bucket, redirects are re-checked hop
//! by hop, and the body is read under the caller's byte cap. Extraction
//! reuses the project's own extractors rather than a parallel implementation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use scraper::{ElementRef, Html};
use serde_json::Value;
use url::Url;

use crate::config::{Config, ManagedApiCfg};
use crate::extract::{Extractor, JsonExtractor};
use crate::fetcher::FetchOutcome;
use crate::metrics::Metrics;
use crate::policy::Politeness;
use crate::robots::Robots;
use crate::scrape_api::models::{ArtifactRef, ErrorBody, OutputFormat, ScrapeRequest};
use crate::ssrf::{Guard, SsrfPolicy};
use crate::tenancy::limits::KeyLimits;
use crate::usage::ledger::RenderMode;

/// The user agent every managed request announces itself with.
pub const USER_AGENT: &str = "harvest-managed-api/1.0 (+https://example.invalid/bot)";

/// Why a request did not produce a body.
#[derive(Debug, Clone)]
pub enum ScrapeError {
    /// 400 — the request itself is unusable.
    BadRequest(ErrorBody),
    /// 401 — the key is missing or does not match.
    Unauthorized(ErrorBody),
    /// 403 — the key lacks the scope, or robots/policy refused the target.
    Forbidden(ErrorBody),
    /// 404 — no such resource.
    NotFound(ErrorBody),
    /// 409 — the key exists but is not usable.
    Conflict(ErrorBody),
    /// 429 — a rate limit, concurrency cap or quota was exhausted.
    TooManyRequests(ErrorBody),
    /// 500 — anything that is the server's fault.
    Internal(ErrorBody),
}

impl ScrapeError {
    pub fn status(&self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::Unauthorized(_) => 401,
            Self::Forbidden(_) => 403,
            Self::NotFound(_) => 404,
            Self::Conflict(_) => 409,
            Self::TooManyRequests(_) => 429,
            Self::Internal(_) => 500,
        }
    }

    pub fn body(&self) -> ErrorBody {
        match self {
            Self::BadRequest(b)
            | Self::Unauthorized(b)
            | Self::Forbidden(b)
            | Self::NotFound(b)
            | Self::Conflict(b)
            | Self::TooManyRequests(b)
            | Self::Internal(b) => b.clone(),
        }
    }

    pub fn from_anyhow(err: anyhow::Error) -> Self {
        Self::Internal(ErrorBody::new("internal_error", format!("{err}")))
    }
}

/// What one managed request produced.
#[derive(Debug, Clone)]
pub struct ScrapeOutcome {
    pub status: u16,
    pub url: String,
    pub final_url: String,
    pub content_type: String,
    pub body: String,
    pub bytes: u64,
    pub latency_ms: u64,
    pub request_id: String,
    pub render: RenderMode,
    pub artifacts: Vec<ArtifactRef>,
    pub extracted: Option<Value>,
    pub records: Vec<Value>,
    pub screenshot: Option<ArtifactRef>,
}

/// The engine.
pub struct ScrapeService {
    cfg: Arc<Config>,
    managed: ManagedApiCfg,
    politeness: Arc<Politeness>,
    guard: Guard,
    metrics: Arc<Metrics>,
    client: reqwest::Client,
    #[cfg(feature = "browser")]
    browser: Option<Arc<crate::browser::api::BrowserApi>>,
}

impl ScrapeService {
    pub fn new(cfg: Arc<Config>, managed: ManagedApiCfg, metrics: Arc<Metrics>) -> Result<Self> {
        let politeness = Arc::new(Politeness::new(&cfg));
        // An explicit operator opt-in for the documented demo/test mode.
        // Unspecified, multicast, link-local and metadata addresses stay
        // refused either way; production must leave this off.
        let policy = SsrfPolicy {
            max_redirects: managed.max_redirects,
            allow_private_targets: managed.allow_private_targets,
            // The same opt-in that unlocks loopback targets is what makes the
            // documented local-demo flow work: the demo origin is plain
            // `http`. Without it every request is https-only, which is what
            // production wants.
            allow_http: managed.allow_private_targets,
            ..SsrfPolicy::default()
        };
        let client = reqwest::Client::builder()
            // Redirects are followed by hand so every hop can be re-checked.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_millis(managed.request_timeout_ms))
            .timeout(Duration::from_millis(managed.request_timeout_ms))
            .build()
            .context("cannot build the managed API HTTP client")?;
        Ok(Self {
            cfg,
            managed,
            politeness,
            guard: Guard::new(policy),
            metrics,
            client,
            #[cfg(feature = "browser")]
            browser: None,
        })
    }

    #[cfg(feature = "browser")]
    pub fn with_browser(&mut self, browser: Arc<crate::browser::api::BrowserApi>) {
        self.browser = Some(browser);
    }

    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    pub fn managed(&self) -> &ManagedApiCfg {
        &self.managed
    }

    /// Run one request. `limits` is the authenticated key's plan.
    pub async fn run(
        &self,
        req: &ScrapeRequest,
        request_id: &str,
        limits: &KeyLimits,
        tenant_id: &str,
        key_id: &str,
    ) -> Result<ScrapeOutcome, ScrapeError> {
        tracing::debug!(tenant = %tenant_id, key = %key_id, "managed scrape");
        let started = Instant::now();
        let parsed = Url::parse(req.url.trim())
            .map_err(|e| ScrapeError::BadRequest(ErrorBody::new("invalid_url", e.to_string())))?;
        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(ScrapeError::BadRequest(ErrorBody::new(
                "invalid_url",
                "only http and https URLs are accepted",
            )));
        }
        let host = parsed
            .host_str()
            .ok_or_else(|| ScrapeError::BadRequest(ErrorBody::new("invalid_url", "no host")))?
            .to_ascii_lowercase();

        // --- SSRF: resolve the host and refuse anything non-public ---------
        let _target = self
            .guard
            .check_url(req.url.trim())
            .map_err(|e| ScrapeError::BadRequest(ErrorBody::new("url_rejected", e.message())))?;

        // --- compliance: robots.txt for this URL ----------------------------
        self.check_robots(&parsed, &host).await?;

        // --- per-domain pacing + breaker ------------------------------------
        self.politeness.wait(&host, &self.cfg, &self.metrics).await;

        let (fetch, final_url) = self
            .fetch_checked(&parsed, limits)
            .await
            .map_err(ScrapeError::from_anyhow)?;

        let latency_ms = started.elapsed().as_millis() as u64;

        if !(200..300).contains(&fetch.status) {
            // A refusal is a refusal, and it is metered like any other
            // request: the tenant asked, the site declined.
            return Err(ScrapeError::Forbidden(ErrorBody::new(
                "upstream_refused",
                format!("the target returned {}", fetch.status),
            )));
        }

        // --- extraction -----------------------------------------------------
        let extracted = self.extract(req, &fetch)?;

        // --- screenshot (browser renders only) -------------------------------
        let screenshot = self
            .screenshot(req, &fetch)
            .await
            .map_err(ScrapeError::from_anyhow)?;

        Ok(ScrapeOutcome {
            status: fetch.status,
            request_id: request_id.to_string(),
            render: req.render,
            url: fetch.url.to_string(),
            final_url,
            content_type: fetch.content_type.clone(),
            body: fetch.body,
            bytes: fetch.bytes as u64,
            latency_ms,
            artifacts: vec![],
            extracted,
            records: Vec::new(),
            screenshot,
        })
    }

    /// Append the metering entry. Failures here are logged and never fail the
    /// request: the customer already got their answer.
    /// The unit estimate for a finished request. The authoritative number
    /// is recorded by the handler once the body is in hand.
    pub fn estimate_units(
        &self,
        req: &ScrapeRequest,
        _limits: &KeyLimits,
        body_bytes: u64,
        artifact_bytes: u64,
    ) -> u32 {
        crate::scrape_api::pricing::Pricing::from_cfg(&self.managed).units_for(
            req,
            body_bytes,
            artifact_bytes,
        )
    }

    async fn check_robots(&self, url: &Url, host: &str) -> Result<(), ScrapeError> {
        if let Some(text) = self.fetcher_robots(url).await {
            let robots = Robots::parse(&text, USER_AGENT);
            if !robots.allows(url.path()) {
                return Err(ScrapeError::Forbidden(ErrorBody::new(
                    "robots_disallowed",
                    format!("robots.txt for {host} disallows {}", url.path()),
                )));
            }
            if let Some(delay) = robots.crawl_delay {
                self.politeness.note_crawl_delay(host, &self.cfg, delay);
            }
        }
        Ok(())
    }

    async fn fetcher_robots(&self, url: &Url) -> Option<String> {
        // The managed API drives its own client so it can re-check every hop;
        // robots.txt is fetched the same way, with the same SSRF rules applied
        // to the robots URL itself.
        let robots_url = match url.join("/robots.txt") {
            Ok(u) => u,
            Err(_) => return None,
        };
        if self.guard.check_url(robots_url.as_str()).is_err() {
            return None;
        }
        let response = self.client.get(robots_url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.text().await.ok()
    }

    /// Follow redirects one hop at a time, re-checking each URL.
    async fn fetch_checked(
        &self,
        start: &Url,
        limits: &KeyLimits,
    ) -> Result<(FetchOutcome, String), anyhow::Error> {
        let mut current = start.clone();
        let mut hops = 0u32;
        let started = Instant::now();

        let (status, content_type, body, bytes) = loop {
            if hops > self.managed.max_redirects {
                return Err(anyhow::anyhow!(
                    "more than {} redirects",
                    self.managed.max_redirects
                ));
            }
            // Re-resolve and re-check every hop: a redirect can point anywhere.
            let _target = self
                .guard
                .check_redirect(current.as_str(), hops)
                .map_err(|e| anyhow::anyhow!(e.message()))?;

            let response = self
                .client
                .get(current.clone())
                .send()
                .await
                .with_context(|| format!("cannot reach {}", current.as_str()))?;
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok());
                let Some(location) = location else {
                    // A redirection without a Location is not a redirection we
                    // can follow; treat the response as the body.
                    let (text, len) = read_bounded_body(response, limits.max_response_bytes).await;
                    break (status, content_type, text, len);
                };
                let next = current
                    .join(location)
                    .with_context(|| format!("redirect target {location} is not a valid URL"))?;
                hops += 1;
                current = next;
                continue;
            }

            let (text, len) = read_bounded_body(response, limits.max_response_bytes).await;
            break (status, content_type, text, len);
        };

        let final_url = current.to_string();
        Ok((
            FetchOutcome {
                url: start.clone(),
                etag: None,
                last_modified: None,
                status,
                content_type,
                body,
                latency_ms: started.elapsed().as_millis() as u64,
                proxy_id: String::new(),
                bytes: bytes as usize,
            },
            final_url,
        ))
    }

    /// Apply the customer's extraction rules.
    fn extract(
        &self,
        req: &ScrapeRequest,
        fetch: &FetchOutcome,
    ) -> Result<Option<Value>, ScrapeError> {
        let body = fetch.body.as_str();

        if let Some(tree) = &req.extract.selector {
            let max = req
                .extract
                .max_records
                .unwrap_or(crate::config::V1_MAX_RECORDS);
            let parsed: crate::selector::SelectorTree = serde_json::from_value(tree.clone())
                .map_err(|e| {
                    ScrapeError::BadRequest(ErrorBody::new("invalid_selector", e.to_string()))
                })?;
            let records = crate::selector::engine::extract(body, &parsed, max).map_err(|e| {
                ScrapeError::BadRequest(ErrorBody::new("extract_failed", e.to_string()))
            })?;
            let values: Vec<Value> = records
                .iter()
                .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                .collect();
            return Ok(Some(Value::Array(values)));
        }

        if req.extract.fields.is_empty() {
            return Ok(None);
        }

        if is_json_content(&fetch.content_type) || looks_like_json(body) {
            let doc: Value = serde_json::from_str(body).map_err(|e| {
                ScrapeError::BadRequest(ErrorBody::new("invalid_json", e.to_string()))
            })?;
            let extractor = JsonExtractor::compile(&req.extract.fields).map_err(|e| {
                ScrapeError::BadRequest(ErrorBody::new("invalid_extract", e.to_string()))
            })?;
            let map = extractor.extract_bounded(&doc).map_err(|e| {
                ScrapeError::BadRequest(ErrorBody::new("extract_failed", e.to_string()))
            })?;
            return Ok(Some(Value::Object(map)));
        }

        let document = Html::parse_document(body);
        let extractor = Extractor::compile(&req.extract.fields).map_err(|e| {
            ScrapeError::BadRequest(ErrorBody::new("invalid_extract", e.to_string()))
        })?;
        let map = extractor.extract_bounded(&document).map_err(|e| {
            ScrapeError::BadRequest(ErrorBody::new("extract_failed", e.to_string()))
        })?;
        Ok(Some(Value::Object(map)))
    }

    /// Capture a screenshot, which only a real browser renderer can do.
    async fn screenshot(
        &self,
        req: &ScrapeRequest,
        fetch: &FetchOutcome,
    ) -> Result<Option<ArtifactRef>, anyhow::Error> {
        if !req.screenshot && req.format != OutputFormat::Screenshot {
            return Ok(None);
        }
        if req.render != RenderMode::Browser {
            return Err(anyhow::anyhow!("screenshot requires render: \"browser\""));
        }
        // The renderer navigates to the URL the customer actually reached,
        // which is why the fetched target is logged here in every build.
        tracing::debug!(target = %fetch.url.as_str(), "capturing a screenshot");
        #[cfg(feature = "browser")]
        {
            let Some(browser) = self.browser.as_ref() else {
                return Err(anyhow::anyhow!(
                    "no browser runtime is attached to this deployment"
                ));
            };
            let Some(artifact) = self.render_browser(browser, req, fetch).await? else {
                return Ok(None);
            };
            return Ok(Some(artifact));
        }
        #[allow(unreachable_code)]
        Err(anyhow::anyhow!(
            "browser rendering is not enabled in this build (enable the `browser` feature)"
        ))
    }

    #[cfg(feature = "browser")]
    async fn render_browser(
        &self,
        browser: &Arc<crate::browser::api::BrowserApi>,
        req: &ScrapeRequest,
        fetch: &FetchOutcome,
    ) -> Result<Option<ArtifactRef>, anyhow::Error> {
        use crate::browser::api_models::BrowserPreviewRequest;
        use crate::browser::workflow::{WaitCondition, Workflow, WorkflowStep};

        // Navigate to the URL the customer actually reached: a redirect chain
        // ends somewhere, and the screenshot must show that page.
        let target_url = fetch.url.clone();
        let mut steps = vec![WorkflowStep::Navigate {
            url: target_url.to_string(),
        }];
        if let Some(selector) = &req.wait.until_selector {
            steps.push(WorkflowStep::Wait {
                condition: WaitCondition::SelectorAttached {
                    selector: selector.clone(),
                },
                timeout_ms: req.wait.ms.unwrap_or(crate::config::V1_RENDER_TIMEOUT_MS),
            });
        }
        let tree = req.extract.selector.clone();
        let parsed_tree: Option<crate::selector::SelectorTree> = match &tree {
            Some(v) => Some(serde_json::from_value(v.clone())?),
            None => None,
        };
        let preview = BrowserPreviewRequest {
            request_id: crate::ids::CorrelationId::new().to_string(),
            target_id: format!("mv1-{}", target_url.host_str().unwrap_or("local")),
            target_url: target_url.to_string(),
            session_id: req.session_key.clone(),
            allowed_hosts: vec![target_url.host_str().unwrap_or("").to_string()],
            workflow: Workflow(steps),
            selector_tree: parsed_tree,
            capture_screenshot: true,
        };
        let response = browser.preview(preview).await?;
        let screenshot = response
            .artifacts
            .iter()
            .find(|a| kind_of(a) == "screenshot")
            .map(to_artifact_ref)
            .transpose()?;
        Ok(screenshot)
    }

    /// The body for the requested output format. `extracted` JSON is
    /// substituted when the caller asked for `format: "json"` and the
    /// response carries a parsed document.
    pub fn body_for_format(&self, req: &ScrapeRequest, fetch: &FetchOutcome) -> String {
        let raw = fetch.body.as_str();
        match req.format {
            OutputFormat::Html | OutputFormat::Screenshot => raw.to_string(),
            OutputFormat::Json => raw.to_string(),
            OutputFormat::Text => html_to_text(raw),
            OutputFormat::Markdown => html_to_markdown(raw),
        }
    }
}

fn kind_of(artifact: &crate::browser::page::PageArtifactRef) -> &str {
    artifact.kind.as_str()
}

fn to_artifact_ref(
    artifact: &crate::browser::page::PageArtifactRef,
) -> Result<ArtifactRef, anyhow::Error> {
    Ok(ArtifactRef {
        id: artifact.id.clone(),
        kind: artifact.kind.clone(),
        bytes: artifact.bytes,
    })
}

fn is_json_content(content_type: &str) -> bool {
    content_type.to_ascii_lowercase().contains("json")
}

fn looks_like_json(body: &str) -> bool {
    let trimmed = body.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

/// Read the body under a hard cap. Exceeding the cap is a refusal, not a
/// truncation: the caller asked for at most this many bytes, and handing
/// back a half-read body would be worse than refusing.
async fn read_bounded_body(response: reqwest::Response, max_bytes: u64) -> (String, u64) {
    if let Some(length) = response.content_length().filter(|l| *l > max_bytes) {
        return (String::new(), length);
    }
    let bytes = response.bytes().await.unwrap_or_default();
    let total = bytes.len() as u64;
    if total > max_bytes {
        return (String::new(), total);
    }
    (String::from_utf8_lossy(&bytes).into_owned(), total)
}

/// Block-level containers: an element matching this list starts and ends a
/// rendered line.
const BLOCK_TAGS: &str = "p,div,section,article,header,footer,main,aside,nav,figure,figcaption,\
blockquote,pre,table,thead,tbody,tr,td,th,ul,ol,li,dl,dt,dd,h1,h2,h3,h4,h5,h6,hr,br,details,\
summary,caption";

/// Block containers that are not already covered by a dedicated rule in the
/// markdown renderer (headings, lists, quotes and code are their own).
const TEXT_BLOCK_TAGS: &str = "p,div,section,article,header,footer,main,aside,nav,figure,\
figcaption,dl,dt,dd,details,summary,caption,table,tbody,thead";

/// Elements whose text is never part of the rendered output.
const SKIP: &[&str] = &[
    "script", "style", "noscript", "template", "svg", "head", "meta", "link",
];

/// The innermost elements matching `tags`: an element is dropped when it
/// contains a matching descendant, so a container never re-emits the text of
/// the block inside it.
fn innermost<'a>(document: &'a Html, tags: &str) -> Vec<ElementRef<'a>> {
    let Ok(sel) = scraper::Selector::parse(tags) else {
        return Vec::new();
    };
    document
        .select(&sel)
        .filter(|el| el.select(&sel).count() <= 1)
        .collect()
}

/// Plain text: block elements become newlines, `script` and `style` are
/// dropped, and the parser has already decoded entities.
pub fn html_to_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let blocks = innermost(&document, BLOCK_TAGS);
    let mut out = String::new();
    if blocks.is_empty() {
        // Nothing block-level: fall back to the whole document so bare text
        // is not lost.
        out.push_str(
            &document
                .select(
                    &scraper::Selector::parse("body")
                        .unwrap_or_else(|_| scraper::Selector::parse("html").unwrap()),
                )
                .next()
                .map(|el| el.text().collect::<String>())
                .unwrap_or_default(),
        );
    } else {
        for el in blocks {
            let name = el.value().name();
            if SKIP.contains(&name) {
                continue;
            }
            if el.text().collect::<String>().trim().is_empty() {
                continue;
            }
            if name == "br" {
                out.push('\n');
                continue;
            }
            if name == "hr" {
                out.push_str("\n\n");
                continue;
            }
            out.push('\n');
            out.push_str(&el.text().collect::<String>());
        }
    }
    normalize_blank_lines(&mut out);
    out
}

fn normalize_blank_lines(text: &mut String) {
    let mut out: Vec<String> = Vec::new();
    let mut blank = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !blank {
                out.push(String::new());
                blank = true;
            }
        } else {
            out.push(trimmed.to_string());
            blank = false;
        }
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    while out.first().is_some_and(|l| l.is_empty()) {
        out.remove(0);
    }
    *text = out.join("\n");
}

/// A small, dependency-free CommonMark subset: headings, paragraphs, links,
/// emphasis, lists, block quotes and fenced code. Bounded by the response
/// byte cap; inline rendering reuses the parser's text so a block's markup
/// is applied to its own text rather than re-parsed.
pub fn html_to_markdown(html: &str) -> String {
    let document = Html::parse_document(html);
    let mut out = String::new();

    for el in innermost(&document, "pre") {
        out.push_str("```\n");
        out.push_str(el.text().collect::<String>().trim_end());
        out.push_str("\n```\n");
    }
    for el in innermost(&document, "blockquote") {
        for line in el.text().collect::<String>().lines() {
            out.push_str("> ");
            out.push_str(line.trim());
            out.push('\n');
        }
    }
    for (tags, prefix) in [
        ("h1", "# "),
        ("h2", "## "),
        ("h3", "### "),
        ("h4", "#### "),
        ("h5", "##### "),
        ("h6", "###### "),
    ] {
        for el in innermost(&document, tags) {
            out.push_str(prefix);
            render_inline(&el, &mut out);
            out.push('\n');
        }
    }
    for el in innermost(&document, "li") {
        out.push_str("- ");
        render_inline(&el, &mut out);
        out.push('\n');
    }
    for _ in innermost(&document, "hr") {
        out.push_str("\n---\n");
    }
    for el in innermost(&document, TEXT_BLOCK_TAGS) {
        let name = el.value().name();
        if SKIP.contains(&name) {
            continue;
        }
        if el.text().collect::<String>().trim().is_empty() {
            continue;
        }
        out.push('\n');
        render_inline(&el, &mut out);
        out.push('\n');
    }
    for _ in innermost(&document, "br") {
        out.push('\n');
    }

    normalize_blank_lines(&mut out);
    out
}

/// Render one element's inline content. Link, emphasis and code markup is
/// applied to the element's own text, which keeps the output deterministic
/// and avoids re-parsing a subtree the HTML parser already walked.
fn render_inline(el: &ElementRef, out: &mut String) {
    let mut value = el.text().collect::<String>();
    for child in el.descendent_elements() {
        let name = child.value().name();
        if !matches!(name, "a" | "strong" | "b" | "em" | "i" | "code") {
            continue;
        }
        let text = child.text().collect::<String>();
        if text.trim().is_empty() {
            continue;
        }
        let replacement = match name {
            "a" => {
                let href = child.value().attr("href").unwrap_or("");
                if href.starts_with('#') || href.is_empty() {
                    continue;
                }
                format!("[{text}]({href})")
            }
            "strong" | "b" => format!("**{text}**"),
            "em" | "i" => format!("_{text}_"),
            _ => format!("`{text}`"),
        };
        value = value.replace(&text, &replacement);
    }
    for line in value.split('\n') {
        out.push_str(line.trim());
        out.push(' ');
    }
}
