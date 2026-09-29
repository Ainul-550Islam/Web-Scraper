# Crawl planning & discovery

`src/crawl/` is a **planner**: it decides what may be requested next and why
everything else may not. It does not fetch, schedule or extract — those remain
the jobs of `fetcher`, `sched`/`supervisor` and `extract`:

| Concern | Owner |
|---|---|
| HTTP, redirects, retries, proxy failover | `src/fetcher.rs` |
| robots.txt, crawl-delay, politeness | `src/robots.rs`, `src/policy.rs` |
| rate limits, concurrency, budgets | `src/sched.rs`, `src/budget.rs` |
| resume | `src/ckpt.rs` |
| extraction, transforms, schema, sinks | `src/extract.rs`, `src/transform.rs`, `src/schema.rs`, `src/sink.rs` |

Nothing in this module relaxes an existing limit; it only refuses more.

## Plan shape (`CrawlCfg`)

```jsonc
{
  "start_urls": ["https://shop.test/catalog"],
  "include": ["/product/"], "exclude": ["/cart"],
  "max_depth": 2, "max_pages": 200, "max_links_per_page": 200,
  "content_types": ["html", "json"],
  "discovery": { "pagination": true, "pagination_selector": "a.next",
                 "canonical": true, "alternates": false, "feeds": false,
                 "documents": true, "media": false, "same_host_only": true,
                 "max_per_kind": 500 },
  "sitemap": true, "sitemap_url": null, "sitemap_max_depth": 1,
  "max_response_bytes": 8388608,
  "media": { "max_bytes": 16777216, "max_resources": 1000, "max_per_page": 100,
             "allow": [], "deny": [], "allow_unknown_size": false },
  "documents_cap": 10000
}
```

`CrawlPlan::from_target` builds the same structure from an existing
`TargetCfg`, reusing the target's own depth, page budget, include/exclude and
sitemap settings instead of a parallel configuration.

## Ceilings (validated in `CrawlPlan::new`)

| Limit | Ceiling |
|---|---|
| `max_depth` | 20 |
| `max_pages` | 1 000 000 |
| `max_links_per_page` | 10 000 |

A missing start URL, a non-http(s) URL or an unknown content-type name is an
error, not a warning.

## Discovery categories

| Category | Source | Notes |
|---|---|---|
| links | `<a href>` | fragment stripped, de-duplicated |
| pagination | `rel="next"` / `rel="prev"`, or an operator CSS selector | **never synthesized** — the engine does not invent `?page=2` |
| canonical | `<link rel="canonical">` | at most one per page |
| alternates | `<link rel="alternate">` (non-feed) | `hreflang`/`type` kept as a hint |
| feeds | `rel="alternate" type="application/rss+xml"` (or atom) | |
| sitemaps | `<link rel="sitemap">`, plus every `Sitemap:` line in robots.txt | index documents return children for bounded recursion |
| documents | `<a href>` ending in pdf/csv/xlsx/json/xml/zip/… | recorded as resources |
| media | `img[src]`, `source[src]`, `video[src]`, `audio[src]` | recorded as resources |

Malformed HTML and malformed XML degrade: `scraper` repairs truncated documents
and the lenient sitemap scanner returns an empty list. Neither panics nor aborts a
run.

## Admission order

`plan.admit(url, depth)` is pure, so the console can preview the effective scope:

1. must be an absolute http(s) URL with a host
2. host in scope (start hosts, plus the sitemap host when configured)
3. robots, once a robots document is loaded (before that, nothing is *assumed*
   allowed — the plan simply cannot check yet)
4. `exclude` (substring, evaluated first) then `include` (if non-empty)
5. depth ≤ `max_depth`
6. pending + visited < `max_pages`
7. not already visited or queued

Every refusal increments a counter in `CrawlStats` — refusals are observable,
never silent.

## Content and resources

* Bodies over `max_response_bytes` are refused **before** decoding, not after.
* Pages whose content type is out of scope are recorded as documents but not
  expanded for links.
* Resources (documents + media) are recorded as **metadata only**: URL, MIME,
  extension, source page, discovery kind. `size` and `sha256` stay `null` until
  something actually fetches the bytes — discovery publishes no checksum it did
  not compute.
* MIME comes from the response header when there is one; an extension guess is
  flagged `mime_guessed: true` (see [content classification](#content-classification)).

## Content classification

`src/content.rs` is the single place MIME/charset decoding is decided:

* a present, non-generic `Content-Type` is authoritative, and a contradiction
  with the sniffed bytes is reported as `mismatch` rather than resolved silently;
* sniffing (bounded to 8 KiB) fills in absent or `application/octet-stream`
  headers;
* charsets: UTF-8/UTF-16 (BOM), ISO-8859-1 and Windows-1252 are transcoded;
  undecodable bytes become U+FFFD;
* CSV is header-driven only — arbitrary text looks like CSV, so guessing it from
  content would mislabel half the web;
* extensions are *reported* (for resource metadata) and never used to decide how
  a fetched body is parsed.

## Tests

`tests/crawl_discovery.rs` (15 tests) covers every discovery category, malformed
HTML/XML, the pagination selector, sitemap + index recursion, robots authority,
include/exclude and host scope, depth and page budgets, duplicate collapsing,
oversized bodies, content-type scope, metadata-only resources, the bounded
document index, configuration validation, `from_target` reuse, and that the plan
snapshot carries no payloads.
