//! Discovery — what a fetched page tells us about what to fetch next.
//!
//! Pure parsing: in goes a page (URL + body), out comes a typed set of
//! candidates grouped by *why* they were found. No fetching, no policy, no
//! scheduling — the planner in [`crate::crawl`] owns those decisions. That
//! split is why this module is trivially testable against malformed HTML and
//! hostile XML (see `tests/crawl_discovery.rs`).
//!
//! Everything discovered is bounded by [`DiscoveryCfg::max_per_kind`]: a page
//! with 50k links yields at most that many candidates per category, and the
//! planner applies the real caps (depth, page budget, robots) on top.
//!
//! Honest limits, stated up front:
//! * pagination is **declared**, not guessed — `rel="next"/"prev"` or an
//!   operator-supplied CSS selector. The engine does not invent `?page=2` URLs,
//!   because inventing URLs means inventing requests a site never published;
//! * link extraction uses the same `scraper` CSS engine as the extractor, so
//!   selectors behave identically to extraction rules;
//! * XML/sitemap handling reuses `crate::sitemap::parse` (the lenient scanner)
//!   and degrades to an empty list on malformed input instead of failing.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::document::DiscoverySource;

/// Extensions that make an `<a href>` a *downloadable document* rather than a
/// page to crawl. Compared case-insensitively against the URL path extension.
pub const DOCUMENT_EXTS: [&str; 14] = [
    "pdf", "csv", "tsv", "xlsx", "xls", "docx", "doc", "json", "xml", "zip", "txt", "pptx",
    "parquet", "rss",
];

/// Media element/attribute pairs scraped for resource discovery.
const MEDIA_SELECTORS: [&str; 5] = [
    "img[src]",
    "source[src]",
    "video[src]",
    "audio[src]",
    "a[href]",
];

/// What discovery is allowed to look for, and how much of it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryCfg {
    /// Hard cap per category per page (links, media, documents, …).
    pub max_per_kind: usize,
    /// Follow `rel="next"` / `rel="prev"` (and `<a rel="next">`).
    pub pagination: bool,
    /// Optional CSS selector for a site's "next page" control, used when the
    /// document does not declare rel=next (WordPress-style `a.next`, …).
    pub pagination_selector: Option<String>,
    /// Collect `<link rel="canonical">`.
    pub canonical: bool,
    /// Collect `<link rel="alternate">` (language/format variants).
    pub alternates: bool,
    /// Collect `<link rel="sitemap">` and RSS/Atom feeds.
    pub feeds: bool,
    /// Collect links whose extension is in [`DOCUMENT_EXTS`].
    pub documents: bool,
    /// Collect `img/source/video/audio` resources.
    pub media: bool,
    /// Restrict every candidate to the start URL's host.
    pub same_host_only: bool,
}

impl Default for DiscoveryCfg {
    fn default() -> Self {
        Self {
            max_per_kind: 500,
            pagination: true,
            pagination_selector: None,
            canonical: true,
            alternates: true,
            feeds: true,
            documents: true,
            media: true,
            same_host_only: true,
        }
    }
}

/// One discovered candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovered {
    /// Absolute URL (fragments stripped).
    pub url: String,
    pub source: DiscoverySource,
    /// `rel` tokens on the element (`next`, `canonical`, `alternate`, …).
    pub rel: Vec<String>,
    /// Link text or `alt` text, bounded — useful as a title hint, never parsed.
    pub label: Option<String>,
    /// `hreflang`/`type` when present on an alternate/sitemap link.
    pub hint: Option<String>,
    /// Depth this candidate would be fetched at (parent depth + 1).
    pub depth: u32,
}

/// A page's complete discovery result, grouped by category. Order within each
/// category is document order (stable, deterministic).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    /// Ordinary navigable links.
    pub links: Vec<Discovered>,
    /// `rel=next/prev` and configured pagination controls.
    pub pagination: Vec<Discovered>,
    /// At most one canonical target per page.
    pub canonical: Option<Discovered>,
    /// `rel=alternate` variants.
    pub alternates: Vec<Discovered>,
    /// RSS/Atom feeds.
    pub feeds: Vec<Discovered>,
    /// Sitemap references declared in the document.
    pub sitemaps: Vec<Discovered>,
    /// Downloadable documents (PDF/CSV/XLSX/…).
    pub documents: Vec<Discovered>,
    /// Images, audio, video.
    pub media: Vec<Discovered>,
}

impl Discovery {
    /// Every distinct URL discovered, in category order (links, pagination,
    /// canonical, alternates, feeds, sitemaps, documents, media).
    pub fn all(&self) -> Vec<&Discovered> {
        let mut out: Vec<&Discovered> = Vec::new();
        out.extend(self.links.iter());
        out.extend(self.pagination.iter());
        if let Some(c) = self.canonical.as_ref() {
            out.push(c);
        }
        out.extend(self.alternates.iter());
        out.extend(self.feeds.iter());
        out.extend(self.sitemaps.iter());
        out.extend(self.documents.iter());
        out.extend(self.media.iter());
        out
    }

    pub fn count(&self) -> usize {
        self.links.len()
            + self.pagination.len()
            + usize::from(self.canonical.is_some())
            + self.alternates.len()
            + self.feeds.len()
            + self.sitemaps.len()
            + self.documents.len()
            + self.media.len()
    }
}

/// Result of parsing one sitemap document: page URLs and/or child sitemaps.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SitemapDiscovery {
    pub urls: Vec<Discovered>,
    /// Child sitemap documents (from a `<sitemapindex>`), to be fetched next.
    pub children: Vec<Discovered>,
    pub is_index: bool,
}

/// Resolve an href found in a document against its URL. Returns `None` for
/// schemes we never request (mailto:, javascript:, data:, tel:, …) and for
/// values that do not resolve at all.
pub fn resolve(base: &Url, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    if let Some(rest) = href.split_once(':') {
        let scheme = rest.0.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "http" | "https") {
            return None;
        }
    }
    let joined = base.join(href).ok()?;
    let mut out = joined.clone();
    out.set_fragment(None);
    if out.scheme() != "http" && out.scheme() != "https" {
        return None;
    }
    Some(out.to_string())
}

/// Substring matching, matching the target `include`/`exclude` semantics
/// already used by `crate::config::TargetCfg` (contains, not regex).
pub fn matches_any(url: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }
    patterns
        .iter()
        .any(|p| !p.is_empty() && url.contains(p.as_str()))
}

fn sel(css: &str) -> Option<scraper::Selector> {
    scraper::Selector::parse(css).ok()
}

fn attr(el: &scraper::ElementRef<'_>, name: &str) -> Option<String> {
    let v = el.value().attr(name)?;
    let v = v.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

fn bounded_label(el: &scraper::ElementRef<'_>) -> Option<String> {
    let text = el.text().collect::<String>();
    let alt = el.value().attr("alt").unwrap_or("").to_string();
    let raw = if text.trim().is_empty() { alt } else { text };
    let label: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        None
    } else {
        Some(label.chars().take(160).collect())
    }
}

fn push_bounded(v: &mut Vec<Discovered>, item: Discovered, cap: usize) {
    if v.len() >= cap {
        return;
    }
    if v.iter()
        .any(|d| d.url == item.url && d.source == item.source)
    {
        return;
    }
    v.push(item);
}

/// Parse an HTML document and discover everything the configuration allows.
/// Malformed HTML is the norm on the web: `scraper` (html5ever) repairs it, so
/// truncated documents simply yield fewer candidates — never an error.
pub fn discover_html(base: &Url, body: &str, cfg: &DiscoveryCfg, depth: u32) -> Discovery {
    let mut out = Discovery::default();
    let doc = scraper::Html::parse_document(body);
    let cap = cfg.max_per_kind.max(1);

    // Canonical + alternates + feeds + sitemap refs: <link> elements.
    if cfg.canonical || cfg.alternates || cfg.feeds {
        if let Some(s) = sel("link[rel]") {
            for el in doc.select(&s) {
                let rel_raw = el.value().attr("rel").unwrap_or("").to_ascii_lowercase();
                let rel: Vec<String> = rel_raw.split_whitespace().map(|r| r.to_string()).collect();
                let Some(href) = attr(&el, "href").and_then(|h| resolve(base, &h)) else {
                    continue;
                };
                if !host_ok(base, &href, cfg.same_host_only) {
                    continue;
                }
                let hint = attr(&el, "hreflang").or_else(|| attr(&el, "type"));
                let item = |source: DiscoverySource| Discovered {
                    url: href.clone(),
                    source,
                    rel: rel.clone(),
                    label: attr(&el, "title"),
                    hint: hint.clone(),
                    depth,
                };
                if cfg.canonical && rel.iter().any(|r| r == "canonical") && out.canonical.is_none()
                {
                    out.canonical = Some(item(DiscoverySource::Canonical));
                } else if cfg.alternates && rel.iter().any(|r| r == "alternate") {
                    let mime = attr(&el, "type").unwrap_or_default();
                    if mime.contains("rss") || mime.contains("atom") {
                        if cfg.feeds {
                            push_bounded(&mut out.feeds, item(DiscoverySource::Feed), cap);
                        }
                    } else {
                        push_bounded(&mut out.alternates, item(DiscoverySource::Alternate), cap);
                    }
                } else if cfg.feeds && rel.iter().any(|r| r == "sitemap") {
                    push_bounded(&mut out.sitemaps, item(DiscoverySource::Sitemap), cap);
                }
            }
        }
    }

    // Pagination: rel=next/prev first, then the operator's selector.
    if cfg.pagination {
        if let Some(s) = sel("link[rel=next], link[rel=prev], a[rel=next], a[rel=prev]") {
            for el in doc.select(&s) {
                let rel = el
                    .value()
                    .attr("rel")
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .split_whitespace()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>();
                let Some(href) = attr(&el, "href").and_then(|h| resolve(base, &h)) else {
                    continue;
                };
                if !host_ok(base, &href, cfg.same_host_only) {
                    continue;
                }
                push_bounded(
                    &mut out.pagination,
                    Discovered {
                        url: href,
                        source: DiscoverySource::Pagination,
                        rel,
                        label: bounded_label(&el),
                        hint: None,
                        depth,
                    },
                    cap,
                );
            }
        }
        if let Some(css) = cfg.pagination_selector.as_deref() {
            if let Some(s) = sel(css) {
                for el in doc.select(&s) {
                    let Some(href) = attr(&el, "href").and_then(|h| resolve(base, &h)) else {
                        continue;
                    };
                    if !host_ok(base, &href, cfg.same_host_only) {
                        continue;
                    }
                    push_bounded(
                        &mut out.pagination,
                        Discovered {
                            url: href,
                            source: DiscoverySource::Pagination,
                            rel: vec!["selector".to_string()],
                            label: bounded_label(&el),
                            hint: Some(css.to_string()),
                            depth,
                        },
                        cap,
                    );
                }
            }
        }
    }

    // Links, documents and media all walk the anchor/resource elements once.
    if let Some(s) = sel("a[href]") {
        for el in doc.select(&s) {
            let Some(href) = attr(&el, "href").and_then(|h| resolve(base, &h)) else {
                continue;
            };
            if !host_ok(base, &href, cfg.same_host_only) {
                continue;
            }
            let ext = crate::media::extension_of(&href);
            let is_doc = cfg.documents
                && ext
                    .as_deref()
                    .is_some_and(|e| DOCUMENT_EXTS.contains(&e.to_ascii_lowercase().as_str()));
            let item = Discovered {
                url: href,
                source: if is_doc {
                    DiscoverySource::Resource
                } else {
                    DiscoverySource::Link
                },
                rel: el
                    .value()
                    .attr("rel")
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .split_whitespace()
                    .map(|r| r.to_string())
                    .collect(),
                label: bounded_label(&el),
                hint: ext,
                depth,
            };
            if is_doc {
                push_bounded(&mut out.documents, item, cap);
            } else {
                push_bounded(&mut out.links, item, cap);
            }
        }
    }

    if cfg.media {
        for css in MEDIA_SELECTORS {
            if css == "a[href]" {
                continue; // anchors are handled above; media comes from elements
            }
            if let Some(s) = sel(css) {
                for el in doc.select(&s) {
                    let Some(src) = attr(&el, "src").and_then(|h| resolve(base, &h)) else {
                        continue;
                    };
                    if !host_ok(base, &src, cfg.same_host_only) {
                        continue;
                    }
                    push_bounded(
                        &mut out.media,
                        Discovered {
                            url: src,
                            source: DiscoverySource::Media,
                            rel: Vec::new(),
                            label: bounded_label(&el),
                            hint: Some(el.value().name().to_string()),
                            depth,
                        },
                        cap,
                    );
                }
            }
        }
    }

    out
}

fn host_ok(base: &Url, candidate: &str, same_host_only: bool) -> bool {
    if !same_host_only {
        return true;
    }
    match Url::parse(candidate) {
        Ok(u) => u.host_str() == base.host_str(),
        Err(_) => false,
    }
}

/// Parse a sitemap document. `<sitemapindex>` children are returned
/// separately so the planner can recurse with its own depth + budget rules.
/// Malformed XML degrades to an empty result (the lenient scanner in
/// `crate::sitemap` never errors).
pub fn discover_sitemap(base: &Url, xml: &str, depth: u32) -> SitemapDiscovery {
    let mut out = SitemapDiscovery::default();
    let parsed = crate::sitemap::parse(xml);
    let cap = 50_000usize; // matches crate::sitemap's own MAX_LOCS
    match parsed {
        crate::sitemap::Sitemap::Urls(urls) => {
            for raw in urls.into_iter().take(cap) {
                let Some(url) = resolve(base, &raw) else {
                    continue;
                };
                push_bounded(
                    &mut out.urls,
                    Discovered {
                        url,
                        source: DiscoverySource::Sitemap,
                        rel: Vec::new(),
                        label: None,
                        hint: None,
                        depth,
                    },
                    cap,
                );
            }
        }
        crate::sitemap::Sitemap::Index(children) => {
            out.is_index = true;
            for raw in children.into_iter().take(cap) {
                let Some(url) = resolve(base, &raw) else {
                    continue;
                };
                push_bounded(
                    &mut out.children,
                    Discovered {
                        url,
                        source: DiscoverySource::SitemapIndex,
                        rel: Vec::new(),
                        label: None,
                        hint: None,
                        depth,
                    },
                    cap,
                );
            }
        }
    }
    out
}

/// Every `Sitemap:` line in a robots.txt document (all of them — RFC 9309
/// allows several). `robots::Robots` keeps only the first, which is enough for
/// politeness but not for discovery.
pub fn sitemaps_from_robots(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.split_once(':') else {
            continue;
        };
        if rest.0.trim().eq_ignore_ascii_case("sitemap") {
            let url = rest.1.trim();
            if !url.is_empty() && (url.starts_with("http://") || url.starts_with("https://")) {
                out.push(url.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://shop.test/catalog/page-1").unwrap()
    }

    #[test]
    fn finds_links_canonical_pagination_and_media() {
        let html = r#"<html><head>
            <link rel="canonical" href="https://shop.test/catalog/page-1">
            <link rel="next" href="https://shop.test/catalog/page-2">
            <link rel="alternate" hreflang="de" href="https://shop.test/de/catalog">
            <link rel="alternate" type="application/rss+xml" href="https://shop.test/feed.xml">
            </head><body>
            <a href="/p/1">One</a><a href="https://shop.test/p/2">Two</a>
            <a href="https://shop.test/files/spec.pdf">Spec</a>
            <a href="mailto:a@b.test">mail</a>
            <img src="/img/one.png" alt="one"><video src="/v/clip.mp4"></video>
            </body></html>"#;
        let d = discover_html(&base(), html, &DiscoveryCfg::default(), 1);
        assert_eq!(
            d.canonical.as_ref().map(|c| c.url.as_str()),
            Some("https://shop.test/catalog/page-1")
        );
        assert_eq!(d.pagination.len(), 1);
        assert_eq!(d.pagination[0].url, "https://shop.test/catalog/page-2");
        assert_eq!(d.alternates.len(), 1);
        assert_eq!(d.feeds.len(), 1);
        assert!(d.links.iter().any(|l| l.url == "https://shop.test/p/1"));
        assert!(d.documents.iter().any(|l| l.url.ends_with("spec.pdf")));
        assert_eq!(d.media.len(), 2, "img + video, no mailto");
        assert!(!d.all().iter().any(|x| x.url.starts_with("mailto:")));
    }

    #[test]
    fn malformed_html_yields_partial_results_never_errors() {
        let html = "<html><body><a href='/ok'>ok</a><a href='/broken<><'";
        let d = discover_html(&base(), html, &DiscoveryCfg::default(), 1);
        assert!(d.links.iter().any(|l| l.url == "https://shop.test/ok"));
    }

    #[test]
    fn sitemap_index_recurses_separately() {
        let xml = r#"<sitemapindex><sitemap><loc>https://shop.test/s1.xml</loc></sitemap>
                     <sitemap><loc>/s2.xml</loc></sitemap></sitemapindex>"#;
        let d = discover_sitemap(&base(), xml, 0);
        assert!(d.is_index);
        assert_eq!(d.children.len(), 2);
        assert_eq!(d.children[1].url, "https://shop.test/s2.xml");
        assert!(d.urls.is_empty());
    }

    #[test]
    fn robots_sitemap_lines_are_all_collected() {
        let txt = "User-agent: *\nDisallow: /private\nSitemap: https://a.test/s1.xml\nSitemap: https://a.test/s2.xml\n";
        assert_eq!(sitemaps_from_robots(txt).len(), 2);
    }

    #[test]
    fn include_exclude_are_substring_matches() {
        assert!(matches_any("https://x.test/a/b", &["/a/".to_string()]));
        assert!(!matches_any("https://x.test/a/b", &["/z/".to_string()]));
        assert!(!matches_any("https://x.test/a/b", &[]));
    }
}
