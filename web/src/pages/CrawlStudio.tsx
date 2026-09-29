/**
 * Crawl Studio — compose a bounded crawl plan and the target config that runs
 * it, and see the engine's own limits before you submit anything.
 *
 * The backend stays authoritative in three ways the page makes visible:
 *  - the scope validator uses the ceilings from `harvest::crawl` (depth 20,
 *    1,000,000 pages, 10,000 links/page) and refuses to present an unsafe plan
 *    as valid;
 *  - politeness/network limits (concurrency, timeouts, retries, rate limit,
 *    robots) are read from `GET /config` and shown read-only — Crawl Studio
 *    composes a plan, it does not edit the running engine;
 *  - the two documents it produces (`crawl-plan.json`, `target.json`) are
 *    exactly the shapes `harvest::crawl::CrawlCfg` and
 *    `harvest::config::TargetCfg` deserialize, so nothing here invents a field.
 *
 * Extraction is composed as a real `TargetCfg.extract` map (css / path / attr /
 * all / transforms) plus a `schema` block — the same structures the engine
 * compiles.
 */
import { useEffect, useMemo, useState } from "react";
import { Button, Card, CopyButton, ErrorState, Field, MetricCard, PageHeader, SelectInput, StatusChip, TextInput, Toggle } from "../components/ui";
import { Page } from "../components/AppShell";
import { FormSection } from "../components/patterns";
import CrawlScopePanel, { validateScope, type CrawlScope } from "../components/CrawlScopePanel";
import { TRANSFORM_OPS } from "../lib/configSchema";
import { apiGet } from "../lib/api";
import { fmtNum } from "../lib/format";
import type { ConfigDocument } from "../lib/types";

const CONTENT_TYPES = ["html", "json", "xml", "csv", "text", "pdf", "binary"] as const;
type ContentTypeId = (typeof CONTENT_TYPES)[number];

const DEFAULT_SCOPE: CrawlScope = {
  startUrls: [],
  include: [],
  exclude: [],
  depth: 1,
  maxPages: 100,
  maxLinksPerPage: 200,
  maxResponseBytes: 8 * 1024 * 1024,
  sameHostOnly: true,
  sitemap: false,
  sitemapUrl: "",
};

interface ExtractField {
  name: string;
  css: string;
  path: string;
  attr: string;
  all: boolean;
  transform: string;
  type: string;
}

const FIELD_TYPES = ["text", "number", "int", "bool", "date", "list", "json"] as const;

/** Serializable shape of `harvest::crawl::CrawlCfg` (src/crawl/mod.rs). */
function crawlPlanJson(scope: CrawlScope, discovery: DiscoveryState, kinds: ContentTypeId[], fields: ExtractField[]) {
  return {
    start_urls: scope.startUrls,
    include: scope.include,
    exclude: scope.exclude,
    max_depth: scope.depth,
    max_pages: scope.maxPages,
    max_links_per_page: scope.maxLinksPerPage,
    content_types: kinds,
    discovery: {
      max_per_kind: discovery.maxPerKind,
      pagination: discovery.pagination,
      pagination_selector: discovery.selector.trim() ? discovery.selector : null,
      canonical: discovery.canonical,
      alternates: discovery.alternates,
      feeds: discovery.feeds,
      documents: discovery.documents,
      media: discovery.media,
      same_host_only: scope.sameHostOnly,
    },
    sitemap: scope.sitemap,
    sitemap_url: scope.sitemapUrl.trim() ? scope.sitemapUrl : null,
    sitemap_max_depth: 1,
    max_response_bytes: scope.maxResponseBytes,
    media: {
      max_bytes: discovery.maxBytes,
      max_resources: 1000,
      max_per_page: discovery.maxPerPage,
      allow: discovery.allowMime,
      deny: [],
      allow_unknown_size: discovery.unknownSize,
    },
    documents_cap: 10_000,
    target: targetJson(scope, fields, discovery.enforce),
    fields: fields.map((f) => ({
      name: f.name,
      css: f.css,
      path: f.path.trim() ? f.path : null,
      attr: f.attr.trim() ? f.attr : null,
      all: f.all,
      transforms: f.transform && f.transform !== "none" ? [{ op: f.transform, args: {} }] : [],
    })),
  };
}

/** Serializable shape of `harvest::config::TargetCfg`. */
function targetJson(scope: CrawlScope, fields: ExtractField[], enforce: string) {
  const extract: Record<string, unknown> = {};
  for (const f of fields) {
    if (!f.name.trim()) continue;
    extract[f.name.trim()] = {
      css: f.css,
      path: f.path.trim() ? f.path : null,
      attr: f.attr.trim() ? f.attr : null,
      all: f.all,
      transforms: f.transform && f.transform !== "none" ? [{ op: f.transform, args: {} }] : [],
    };
  }
  const schema_fields: Record<string, string> = {};
  for (const f of fields) {
    if (f.name.trim() && f.type && f.type !== "auto") schema_fields[f.name.trim()] = f.type;
  }
  return {
    id: "crawl-target",
    start_url: scope.startUrls[0] ?? "",
    max_depth: scope.depth,
    follow_links: scope.depth > 0,
    link_selector: null,
    include: scope.include,
    exclude: scope.exclude,
    extract,
    max_pages: scope.maxPages,
    mode: "html",
    sitemap: scope.sitemap,
    sitemap_url: scope.sitemapUrl.trim() ? scope.sitemapUrl : null,
    schema: { enforce, fields: schema_fields, quarantine_path: null },
  };
}

interface DiscoveryState {
  pagination: boolean;
  selector: string;
  canonical: boolean;
  alternates: boolean;
  feeds: boolean;
  documents: boolean;
  media: boolean;
  maxPerKind: number;
  maxBytes: number;
  maxPerPage: number;
  unknownSize: boolean;
  allowMime: string[];
  enforce: string;
}

const DEFAULT_DISCOVERY: DiscoveryState = {
  pagination: true,
  selector: "",
  canonical: true,
  alternates: false,
  feeds: false,
  documents: true,
  media: false,
  maxPerKind: 500,
  maxBytes: 16 * 1024 * 1024,
  maxPerPage: 100,
  unknownSize: false,
  allowMime: [],
  enforce: "off",
};

export default function CrawlStudio() {
  const [scope, setScope] = useState<CrawlScope>(DEFAULT_SCOPE);
  const [discovery, setDiscovery] = useState<DiscoveryState>(DEFAULT_DISCOVERY);
  const [kinds, setKinds] = useState<ContentTypeId[]>(["html", "json"]);
  const [fields, setFields] = useState<ExtractField[]>([
    { name: "title", css: "h1", path: "", attr: "", all: false, transform: "trim", type: "text" },
  ]);
  const [cfg, setCfg] = useState<ConfigDocument | null>(null);
  const [cfgError, setCfgError] = useState<Error | null>(null);
  const [cfgLoading, setCfgLoading] = useState(true);

  // The engine's politeness/network limits are read-only here: they are the
  // authority for what a crawl may actually do.
  useEffect(() => {
    let alive = true;
    setCfgLoading(true);
    apiGet<ConfigDocument>("/config")
      .then((d) => {
        if (!alive) return;
        setCfg(d);
        setCfgError(null);
      })
      .catch((e: unknown) => {
        if (!alive) return;
        setCfg(null);
        setCfgError(e instanceof Error ? e : new Error(String(e)));
      })
      .finally(() => {
        if (alive) setCfgLoading(false);
      });
    return () => {
      alive = false;
    };
  }, []);

  const issues = useMemo(() => validateScope(scope), [scope]);
  const errors = issues.filter((i) => i.level === "error");
  const warns = issues.filter((i) => i.level === "warn");
  const plan = useMemo(() => JSON.stringify(crawlPlanJson(scope, discovery, kinds, fields), null, 2), [scope, discovery, kinds, fields]);

  const unsupported = kinds.filter((k) => k === "pdf" || k === "binary");
  const setD = <K extends keyof DiscoveryState>(k: K, v: DiscoveryState[K]) => setDiscovery((p) => ({ ...p, [k]: v }));

  return (
    <Page>
      <PageHeader
        title="Crawl Studio"
        sub="Compose a bounded crawl plan and the target that runs it — the engine keeps authority over politeness, robots, retries and every size limit."
      />

      <div className="grid grid-metrics">
        <MetricCard
          label="Scope"
          value={<StatusChip status={errors.length ? "error" : warns.length ? "warn" : "ok"} dot label={errors.length ? "invalid" : warns.length ? "warnings" : "valid"} />}
          meta={`${fmtNum(scope.startUrls.length)} start URL(s) · depth ${scope.depth}`}
        />
        <MetricCard label="Page budget" value={fmtNum(scope.maxPages)} unit="pages" meta={`≤ ${fmtNum(scope.maxLinksPerPage)} link(s) per page`} />
        <MetricCard label="Content types" value={fmtNum(kinds.length)} meta={kinds.join(", ") || "none selected"} />
        <MetricCard
          label="Engine limits"
          value={cfg ? `${fmtNum(cfg.concurrency)} workers` : cfgLoading ? "loading…" : "unavailable"}
          meta={cfg ? `retries ${cfg.max_retries} · ${cfg.request_timeout_secs}s timeout · robots ${cfg.respect_robots ? "on" : "off"}` : "connect to see authoritative limits"}
        />
      </div>

      {cfgError ? <Card title="Engine limits"><ErrorState error={cfgError} /></Card> : null}

      <Card title="Plan">
        <CrawlScopePanel scope={scope} onChange={setScope} engineMaxPages={cfg?.max_pages ?? null} />
        {errors.length > 0 ? (
          <ul className="cs-errors" aria-label="plan errors">
            {errors.map((e) => (
              <li key={`${e.field}-${e.message}`}>{e.message}</li>
            ))}
          </ul>
        ) : null}
      </Card>

      <Card title="Discovery">
        <div className="cs-toggles">
          <span className="cs-toggle">
            <Toggle checked={discovery.pagination} onChange={(v) => setD("pagination", v)} label="Pagination links" />
            <span>Pagination (rel=next/prev)</span>
          </span>
          <span className="cs-toggle">
            <Toggle checked={discovery.canonical} onChange={(v) => setD("canonical", v)} label="Canonical links" />
            <span>Canonical links</span>
          </span>
          <span className="cs-toggle">
            <Toggle checked={discovery.alternates} onChange={(v) => setD("alternates", v)} label="Alternate representations" />
            <span>Alternates</span>
          </span>
          <span className="cs-toggle">
            <Toggle checked={discovery.feeds} onChange={(v) => setD("feeds", v)} label="Feeds" />
            <span>RSS/Atom feeds</span>
          </span>
          <span className="cs-toggle">
            <Toggle checked={discovery.documents} onChange={(v) => setD("documents", v)} label="Document links" />
            <span>Documents (pdf/csv/xlsx…)</span>
          </span>
          <span className="cs-toggle">
            <Toggle checked={discovery.media} onChange={(v) => setD("media", v)} label="Media resources" />
            <span>Media (img/video/audio)</span>
          </span>
        </div>
        <div className="cs-grid">
          <Field label="Pagination selector" help="Optional CSS selector when a site does not declare rel=next.">
            <TextInput aria-label="pagination selector" value={discovery.selector} onChange={(e) => setD("selector", e.target.value)} placeholder="a.next" disabled={!discovery.pagination} />
          </Field>
          <Field label="Candidates per kind / page">
            <TextInput aria-label="max per kind" type="number" min={1} value={discovery.maxPerKind} onChange={(e) => setD("maxPerKind", Math.max(1, Number(e.target.value) || 1))} />
          </Field>
          <Field label="Resource byte cap" help="Resources above this are refused; discovery records metadata only.">
            <TextInput aria-label="resource byte cap" type="number" min={1} value={discovery.maxBytes} onChange={(e) => setD("maxBytes", Math.max(1, Number(e.target.value) || 1))} />
          </Field>
          <Field label="Resources per page">
            <TextInput aria-label="resources per page" type="number" min={1} value={discovery.maxPerPage} onChange={(e) => setD("maxPerPage", Math.max(1, Number(e.target.value) || 1))} />
          </Field>
        </div>
        <span className="cs-toggle">
          <Toggle checked={discovery.unknownSize} onChange={(v) => setD("unknownSize", v)} label="Record resources of unknown size" />
          <span>Record resources whose size is not yet known (no bytes are fetched)</span>
        </span>
      </Card>

      <Card title="Content types">
        <div className="cs-kinds">
          {CONTENT_TYPES.map((k) => (
            <label key={k} className="cs-kind">
              <input
                type="checkbox"
                checked={kinds.includes(k)}
                onChange={(e) => setKinds((prev) => (e.target.checked ? [...prev, k] : prev.filter((x) => x !== k)))}
              />
              <span>{k}</span>
            </label>
          ))}
        </div>
        {unsupported.length > 0 ? (
          <p className="t-caption t-warn">
            {unsupported.join(" and ")} are recorded as resources, not parsed into records — selecting them widens
            discovery without producing rows.
          </p>
        ) : null}
        {kinds.length === 0 ? <p className="t-caption t-err">No content type selected: no page will be treated as a record source.</p> : null}
      </Card>

      <Card title="Extraction">
        <div className="cs-fields">
          {fields.map((f, i) => (
            <div className="cs-field" key={i}>
              <TextInput aria-label={`field ${i + 1} name`} value={f.name} placeholder="field name" onChange={(e) => setFields((p) => p.map((x, j) => (j === i ? { ...x, name: e.target.value } : x)))} />
              <TextInput aria-label={`field ${i + 1} css selector`} value={f.css} placeholder="css selector" onChange={(e) => setFields((p) => p.map((x, j) => (j === i ? { ...x, css: e.target.value } : x)))} />
              <TextInput aria-label={`field ${i + 1} jsonpath`} value={f.path} placeholder="$.data.items[*].name" onChange={(e) => setFields((p) => p.map((x, j) => (j === i ? { ...x, path: e.target.value } : x)))} />
              <SelectInput aria-label={`field ${i + 1} transform`} value={f.transform} onChange={(e) => setFields((p) => p.map((x, j) => (j === i ? { ...x, transform: e.target.value } : x)))}>
                <option value="none">no transform</option>
                {TRANSFORM_OPS.map((op) => (
                  <option key={op} value={op}>
                    {op}
                  </option>
                ))}
              </SelectInput>
              <SelectInput aria-label={`field ${i + 1} declared type`} value={f.type} onChange={(e) => setFields((p) => p.map((x, j) => (j === i ? { ...x, type: e.target.value } : x)))}>
                <option value="auto">auto</option>
                {FIELD_TYPES.map((t) => (
                  <option key={t} value={t}>
                    {t}
                  </option>
                ))}
              </SelectInput>
              <Button variant="ghost" className="sm" onClick={() => setFields((p) => p.filter((_, j) => j !== i))} aria-label={`remove field ${i + 1}`}>
                Remove
              </Button>
            </div>
          ))}
        </div>
        <div className="cs-actions">
          <Button variant="ghost" onClick={() => setFields((p) => [...p, { name: "", css: "", path: "", attr: "", all: false, transform: "none", type: "auto" }])}>
            Add field
          </Button>
          <Field label="Schema enforcement">
            <SelectInput aria-label="schema enforcement" value={discovery.enforce} onChange={(e) => setD("enforce", e.target.value)}>
              <option value="off">off</option>
              <option value="coerce">coerce</option>
              <option value="strict">strict (quarantine failures)</option>
            </SelectInput>
          </Field>
        </div>
      </Card>

      <Card title="Output" actions={<CopyButton text={plan} label="Copy plan JSON" />}>
        <FormSection id="plan-json" title="crawl-plan.json + target fragment" defaultOpen>
          <pre className="cs-plan" aria-label="crawl plan json">
            {plan}
          </pre>
        </FormSection>
        <p className="t-caption t-muted">
          Save as a config document and run it with <code>harvest run --config &lt;file&gt;</code>; the console never
          starts a crawl on your behalf, and it cannot relax the engine's politeness limits.
        </p>
        <Button
          variant="primary"
          disabled={errors.length > 0}
          onClick={() => {
            const blob = new Blob([plan], { type: "application/json" });
            const url = URL.createObjectURL(blob);
            const a = document.createElement("a");
            a.href = url;
            a.download = "crawl-plan.json";
            a.click();
            URL.revokeObjectURL(url);
          }}
        >
          Download crawl-plan.json
        </Button>
      </Card>

      <style>{`
        .cs-toggles { display: flex; flex-wrap: wrap; gap: 16px; }
        .cs-toggle { align-items: center; display: inline-flex; font-size: 12px; gap: 8px; }
        .cs-grid { display: grid; gap: 12px; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); margin-top: 12px; }
        .cs-kinds { display: flex; flex-wrap: wrap; gap: 14px; }
        .cs-kind { align-items: center; display: inline-flex; font-size: 12px; gap: 6px; }
        .cs-fields { display: flex; flex-direction: column; gap: 8px; }
        .cs-field { display: grid; gap: 8px; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); }
        .cs-actions { align-items: flex-end; display: flex; flex-wrap: wrap; gap: 16px; margin-top: 12px; }
        .cs-plan { background: var(--surface-2); border: 1px solid var(--line); border-radius: var(--r-sm); font-size: 11px;
          max-height: 320px; overflow: auto; padding: 10px; }
        .cs-errors { color: var(--err-ink); font-size: 12px; margin: 12px 0 0; padding-left: 18px; }
      `}</style>
    </Page>
  );
}
