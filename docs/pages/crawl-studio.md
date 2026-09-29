# Crawl Studio

| | |
|---|---|
| Route | `/crawl` |
| Nav group | Delivery |
| Shortcut | — (none registered) |
| Endpoints | `GET /config` (read-only engine limits) |
| Polling | none — fetched once on mount |

Compose a bounded crawl plan and the target config that runs it. Crawl Studio
**composes documents**; it does not start crawls, and it cannot relax a
politeness, robots, retry or size limit — those live in the engine config and are
displayed read-only from `GET /config`.

## Sections

| Section | Controls | Maps to |
|---|---|---|
| Scope | start URLs, include/exclude, depth, page limit, links/page, host restriction, sitemap | `harvest::crawl::CrawlCfg` |
| Discovery | pagination (rel=next/prev + optional selector), canonical, alternates, feeds, documents, media, per-kind cap, resource byte cap, resources/page | `harvest::crawl::discovery::DiscoveryCfg`, `harvest::media::MediaPolicy` |
| Content | html, json, xml, csv, text, pdf, binary | `CrawlCfg::content_types` |
| Limits | response byte cap (plan) + concurrency, timeouts, retries, robots (engine, read-only) | `CrawlCfg::max_response_bytes` · `GET /config` |
| Extraction | field name, CSS selector or JSONPath, transform op, declared type, schema enforcement | `TargetCfg.extract` · `TargetCfg.schema` |

## Validation uses the engine's own ceilings

The scope validator mirrors `src/crawl/mod.rs`, so an unsafe plan is *refused*,
not quietly rewritten:

| Limit | Ceiling | Source |
|---|---|---|
| depth | 20 | `crawl::MAX_DEPTH_CEILING` |
| pages | 1 000 000 | `crawl::MAX_PAGES_CEILING` |
| links per page | 10 000 | `crawl::MAX_LINKS_PER_PAGE_CEILING` |
| response bytes | 64 MiB warning | `crawl::CrawlCfg::max_response_bytes` |

Warnings (non-blocking) cover: page budgets above 10 000 without a matching rate
limit, depths above 6, and include/exclude overlap — because exclude is evaluated
first in the planner, an overlapping pattern means those URLs never crawl.

## Honest scope

* **Pagination is declared, never invented.** `rel="next"`/`rel="prev"` and an
  optional operator-supplied CSS selector only. The engine does not synthesize
  `?page=2` URLs, because a URL a site never published is a request it never
  agreed to receive.
* **Media/document discovery records metadata, not bytes.** Resources are
  indexed (URL, MIME, extension, source page, discovery kind); size and checksum
  stay `null` until something actually fetches them, and the resource byte cap is
  enforced either way.
* **PDF and binary are resource kinds, not record sources.** Selecting them
  widens discovery; the page says they will not produce rows.
* `GET /config` failing surfaces the normalized API error inline — the limits
  section then reads "unavailable" instead of showing invented numbers.

## Output

Two documents, both downloadable and copyable, in exactly the shapes the engine
deserializes: `crawl-plan.json` (`CrawlCfg`) and the embedded `target` fragment
(`TargetCfg`, including `extract` and `schema`). Apply with
`harvest run --config <file>`; the console never starts a run on your behalf.

- **Source:** `web/src/pages/CrawlStudio.tsx`, `web/src/components/CrawlScopePanel.tsx`
- **Tests:** `web/src/test/crawl-studio.test.tsx` (12 tests: validation, toggles, content types, plan composition, engine limits, backend failure, field rows, responsive layout) · `tests/crawl_discovery.rs` (15 tests on the planner itself)
- **See also:** [configuration studio](config.md) · [pipeline](pipeline.md) · [docs/crawl-discovery.md](../crawl-discovery.md)
