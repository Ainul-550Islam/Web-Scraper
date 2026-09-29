# Module `crawl`

Source: [`src/crawl/mod.rs`](../../src/crawl/mod.rs) · Source: [`src/crawl/discovery.rs`](../../src/crawl/discovery.rs)

Public surface (23 items):

**src/crawl/mod.rs**

- `const MAX_DEPTH_CEILING` — Upper bound we accept for `max_depth`. A deeper crawl is almost always a
  <!-- pub const MAX_DEPTH_CEILING: u32 = 20; -->
- `const MAX_PAGES_CEILING` — Upper bound for `max_pages` (per crawl plan).
  <!-- pub const MAX_PAGES_CEILING: usize = 1_000_000; -->
- `const MAX_LINKS_PER_PAGE_CEILING` — Upper bound for per-page link expansion.
  <!-- pub const MAX_LINKS_PER_PAGE_CEILING: usize = 10_000; -->
- `const DEFAULT_CONTENT_TYPES` — The content types a crawl is allowed to treat as record sources. Anything
  <!-- pub const DEFAULT_CONTENT_TYPES: [ContentKind; 5] = [ -->
- `enum CrawlError`
  <!-- pub enum CrawlError -->
- `struct CrawlCfg`
  <!-- pub struct CrawlCfg -->
- `struct PlannedUrl`
  <!-- pub struct PlannedUrl -->
- `enum Skip`
  <!-- pub enum Skip -->
- `enum Admit`
  <!-- pub enum Admit -->
- `struct Expansion`
  <!-- pub struct Expansion -->
- `struct CrawlStats`
  <!-- pub struct CrawlStats -->
- `struct CrawlPlan`
  <!-- pub struct CrawlPlan -->
- `struct SitemapExpansion`
  <!-- pub struct SitemapExpansion -->

**src/crawl/discovery.rs**

- `const DOCUMENT_EXTS` — Extensions that make an `<a href>` a *downloadable document* rather than a
  <!-- pub const DOCUMENT_EXTS: [&str; 14] = [ -->
- `struct DiscoveryCfg`
  <!-- pub struct DiscoveryCfg -->
- `struct Discovered`
  <!-- pub struct Discovered -->
- `struct Discovery`
  <!-- pub struct Discovery -->
- `struct SitemapDiscovery`
  <!-- pub struct SitemapDiscovery -->
- `fn resolve` — Resolve an href found in a document against its URL. Returns `None` for
  <!-- pub fn resolve(base: &Url, href: &str) -> Option<String> -->
- `fn matches_any` — Substring matching, matching the target `include`/`exclude` semantics
  <!-- pub fn matches_any(url: &str, patterns: &[String]) -> bool -->
- `fn discover_html` — Parse an HTML document and discover everything the configuration allows.
  <!-- pub fn discover_html(base: &Url, body: &str, cfg: &DiscoveryCfg, depth: u32) -> Discovery -->
- `fn discover_sitemap` — Parse a sitemap document. `<sitemapindex>` children are returned
  <!-- pub fn discover_sitemap(base: &Url, xml: &str, depth: u32) -> SitemapDiscovery -->
- `fn sitemaps_from_robots` — Every `Sitemap:` line in a robots.txt document (all of them — RFC 9309
  <!-- pub fn sitemaps_from_robots(text: &str) -> Vec<String> -->

