/**
 * CrawlScopePanel — the effective crawl scope, with the warnings attached.
 *
 * Every warning here is a *statement about the backend's own limits*, not a
 * UI opinion: the ceiling constants mirror `harvest::crawl` (MAX_DEPTH_CEILING
 * 20, MAX_PAGES_CEILING 1_000_000, MAX_LINKS_PER_PAGE_CEILING 10_000). The
 * panel never rewrites a value to make it safe — it refuses to present an
 * unsafe plan as valid, and says which limit applies.
 */
import { InlineError, Field, NumberInput, TagInput, Toggle, StatusChip } from "./ui";

/** Bounds mirrored from `src/crawl/mod.rs` — the backend is authoritative. */
export const DEPTH_CEILING = 20;
export const PAGES_CEILING = 1_000_000;
export const LINKS_PER_PAGE_CEILING = 10_000;
export const RESPONSE_BYTES_CEILING = 64 * 1024 * 1024;

export interface CrawlScope {
  startUrls: string[];
  include: string[];
  exclude: string[];
  depth: number;
  maxPages: number;
  maxLinksPerPage: number;
  maxResponseBytes: number;
  sameHostOnly: boolean;
  sitemap: boolean;
  sitemapUrl: string;
}

export interface ScopeIssue {
  field: string;
  /** Short, factual statement of what is wrong. */
  message: string;
  /** "error" blocks a plan; "warn" is a strong caution. */
  level: "error" | "warn";
}

/** Validate a scope against the backend's own ceilings + plain contradictions. */
// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export function validateScope(s: CrawlScope): ScopeIssue[] {
  const out: ScopeIssue[] = [];
  const httpish = (u: string) => /^https?:\/\/[^\s/]+\/?\S*$/i.test(u.trim());

  if (s.startUrls.length === 0) {
    out.push({ field: "startUrls", message: "at least one start URL is required", level: "error" });
  }
  for (const u of s.startUrls) {
    if (!httpish(u)) {
      out.push({ field: "startUrls", message: `not an http(s) URL: ${u || "(empty)"}`, level: "error" });
    }
  }
  if (s.depth > DEPTH_CEILING) {
    out.push({ field: "depth", message: `depth ${s.depth} exceeds the engine ceiling (${DEPTH_CEILING})`, level: "error" });
  }
  if (s.maxPages > PAGES_CEILING) {
    out.push({ field: "maxPages", message: `page limit exceeds the engine ceiling (${PAGES_CEILING.toLocaleString()})`, level: "error" });
  }
  if (s.maxLinksPerPage > LINKS_PER_PAGE_CEILING) {
    out.push({ field: "maxLinksPerPage", message: `per-page link cap exceeds the engine ceiling (${LINKS_PER_PAGE_CEILING.toLocaleString()})`, level: "error" });
  }
  if (s.maxResponseBytes > RESPONSE_BYTES_CEILING) {
    out.push({ field: "maxResponseBytes", message: "response cap above 64 MiB will be refused by the planner", level: "warn" });
  }
  if (s.maxPages > 10_000) {
    out.push({ field: "maxPages", message: "large page budgets need a matching rate limit — check politeness settings", level: "warn" });
  }
  if (s.depth > 6) {
    out.push({ field: "depth", message: "deep crawls rarely add records and cost politeness budget", level: "warn" });
  }
  // include ∩ exclude: a URL both must and must not be crawled is refused by
  // the planner (exclude is evaluated first), so it is a configuration bug.
  const conflict = s.include.filter((i) => s.exclude.some((e) => e.length > 0 && (i.includes(e) || e.includes(i))));
  if (conflict.length > 0) {
    out.push({
      field: "include",
      message: `include/exclude overlap on ${conflict.length} pattern(s) — exclude wins, so those URLs never crawl`,
      level: "warn",
    });
  }
  if (s.sitemap && s.sitemapUrl.trim().length > 0 && !httpish(s.sitemapUrl)) {
    out.push({ field: "sitemapUrl", message: "sitemap URL is not an http(s) URL", level: "error" });
  }
  return out;
}

export interface Props {
  scope: CrawlScope;
  onChange: (next: CrawlScope) => void;
  /** Engine-side limit read from GET /config, when connected. */
  engineMaxPages?: number | null;
  disabled?: boolean;
}

export default function CrawlScopePanel({ scope, onChange, engineMaxPages = null, disabled = false }: Props) {
  const issues = validateScope(scope);
  const errors = issues.filter((i) => i.level === "error");
  const warns = issues.filter((i) => i.level === "warn");
  const set = <K extends keyof CrawlScope>(key: K, value: CrawlScope[K]) => onChange({ ...scope, [key]: value });
  const issueFor = (field: string) => issues.find((i) => i.field === field) ?? null;

  return (
    <div className="csp">
      <Field label="Start URLs" help="Absolute http(s) URLs. The first one scopes the crawl when host-restricted.">
        <TagInput
          id="start-urls"
          values={scope.startUrls}
          onChange={(v) => set("startUrls", v)}
          placeholder="https://example.test/catalog"
        />
      </Field>

      <div className="csp-grid">
        <Field label="Include patterns" help="Substring match, like TargetCfg.include. Empty = no filter.">
          <TagInput id="include-patterns" values={scope.include} onChange={(v) => set("include", v)} placeholder="/catalog" />
        </Field>
        <Field label="Exclude patterns" help="Evaluated before include; exclude always wins.">
          <TagInput id="exclude-patterns" values={scope.exclude} onChange={(v) => set("exclude", v)} placeholder="/cart" />
        </Field>
      </div>

      <div className="csp-grid">
        <Field
          label="Depth"
          help={`Levels below a start URL (0 = start URLs only, ceiling ${DEPTH_CEILING}).`}
          error={issueFor("depth")?.message ?? null}
        >
          <NumberInput value={scope.depth} onValue={(v) => set("depth", v ?? 0)} min={0} max={DEPTH_CEILING} invalid={!!issueFor("depth")} />
        </Field>
        <Field
          label="Page limit"
          help={`Total pages for the plan (ceiling ${PAGES_CEILING.toLocaleString()}).`}
          error={issueFor("maxPages")?.message ?? null}
        >
          <NumberInput value={scope.maxPages} onValue={(v) => set("maxPages", v ?? 1)} min={1} invalid={!!issueFor("maxPages")} />
        </Field>
        <Field
          label="Links per page"
          help="Candidates taken from one page."
          error={issueFor("maxLinksPerPage")?.message ?? null}
        >
          <NumberInput
            value={scope.maxLinksPerPage}
            onValue={(v) => set("maxLinksPerPage", v ?? 1)}
            min={1}
            invalid={!!issueFor("maxLinksPerPage")}
          />
        </Field>
        <Field
          label="Max response bytes"
          help="Bodies larger than this are refused before decoding."
          error={issueFor("maxResponseBytes")?.message ?? null}
        >
          <NumberInput value={scope.maxResponseBytes} onValue={(v) => set("maxResponseBytes", v ?? 1)} min={1} unit="bytes" />
        </Field>
      </div>

      <div className="csp-toggles">
        <span className="csp-toggle">
          <Toggle checked={scope.sameHostOnly} onChange={(v) => set("sameHostOnly", v)} label="Restrict to start-host" disabled={disabled} />
          <span>Restrict to start host</span>
        </span>
        <span className="csp-toggle">
          <Toggle checked={scope.sitemap} onChange={(v) => set("sitemap", v)} label="Seed from sitemap" disabled={disabled} />
          <span>Seed from sitemap</span>
        </span>
        {scope.sitemap ? (
          <Field label="Sitemap URL" help="Defaults to <origin>/sitemap.xml when blank.">
            <input
              className="input"
              aria-label="sitemap url"
              value={scope.sitemapUrl}
              onChange={(e) => set("sitemapUrl", e.target.value)}
              placeholder="https://example.test/sitemap.xml"
            />
          </Field>
        ) : null}
      </div>

      <div className="csp-state" role="status">
        <StatusChip
          status={errors.length > 0 ? "error" : warns.length > 0 ? "warn" : "ok"}
          dot
          label={errors.length > 0 ? "invalid scope" : warns.length > 0 ? "valid with warnings" : "valid scope"}
        />
        <span className="t-caption t-muted">
          effective: {scope.startUrls.length} start URL(s) · depth {scope.depth} · ≤ {scope.maxPages.toLocaleString()} page(s)
          {engineMaxPages !== null ? ` · engine global limit ${engineMaxPages.toLocaleString()}` : ""}
        </span>
      </div>

      {errors.length === 0 && warns.length > 0 ? (
        <InlineError error={null} />
      ) : null}
      {warns.length > 0 ? (
        <ul className="csp-warns" aria-label="scope warnings">
          {warns.map((w) => (
            <li key={`${w.field}-${w.message}`}>{w.message}</li>
          ))}
        </ul>
      ) : null}
      {errors.length > 0 ? (
        <ul className="csp-errors" aria-label="scope errors">
          {errors.map((w) => (
            <li key={`${w.field}-${w.message}`}>{w.message}</li>
          ))}
        </ul>
      ) : null}

      <style>{`
        .csp { display: flex; flex-direction: column; gap: 14px; }
        .csp-grid { display: grid; gap: 12px; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); }
        .csp-toggles { align-items: center; display: flex; flex-wrap: wrap; gap: 18px; }
        .csp-toggle { align-items: center; display: inline-flex; gap: 8px; font-size: 12px; }
        .csp-state { align-items: center; display: flex; flex-wrap: wrap; gap: 10px; }
        .csp-warns, .csp-errors { font-size: 12px; margin: 0; padding-left: 18px; }
        .csp-warns { color: var(--warn-ink); }
        .csp-errors { color: var(--err-ink); }
      `}</style>
    </div>
  );
}
