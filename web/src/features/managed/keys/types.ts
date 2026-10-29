// types — the wire shapes of the managed scrape API's operator half.
// These are the documents `GET/POST /v1/admin/*` return; nothing here is
// invented, and no field in this file ever carries a key plaintext.

export const RESPONSE_SCHEMA = 1;

export const SCOPE_LABELS: Record<string, string> = {
  scrape: "Scrape",
  jobs_read: "Read jobs",
  jobs_write: "Write jobs",
  usage_read: "Read usage",
  keys_admin: "Administer keys",
};

export const SCOPE_ORDER = [
  "scrape",
  "jobs_read",
  "jobs_write",
  "usage_read",
  "keys_admin",
] as const;

export type ScopeName = (typeof SCOPE_ORDER)[number];

export const KEY_STATUS_LABELS: Record<string, string> = {
  active: "Active",
  suspended: "Suspended",
  revoked: "Revoked",
};

export interface KeyLimits {
  rps: number;
  burst: number;
  max_concurrent_jobs: number;
  monthly_quota: number;
  max_response_bytes: number;
  allowed_domains: string[];
  scopes: string[];
}

/** The persisted key record. `key_hash` is the only secret-adjacent field and
 *  it is a digest: it is never displayed and never round-trips to the server. */
export interface ApiKey {
  id: string;
  tenant_id: string;
  key_hash: string;
  prefix: string;
  scopes: string[];
  limits: KeyLimits;
  status: "active" | "suspended" | "revoked";
  expires_at: number | null;
  created_at: number;
  last_used_at: number | null;
}

/** What `POST /v1/admin/keys` and `…/rotate` return. The plaintext exists in
 *  this response and nowhere else. */
export interface KeyIssued {
  schema: number;
  api_key: string;
  key: ApiKey;
}

export interface Tenant {
  id: string;
  name: string;
  status: string;
  created_at: number;
}

export interface TenantList {
  schema: number;
  tenant_count: number;
  key_count: number;
  tenants: Tenant[];
}

export interface KeyList {
  schema: number;
  keys: ApiKey[];
}

export interface DayRow {
  day: string;
  requests: number;
  units: number;
  bytes: number;
}

export interface UsageRow {
  key_id: string;
  tenant_id: string;
  requests: number;
  ok: number;
  refused: number;
  errors: number;
  units: number;
  bytes: number;
  render_none: number;
  render_browser: number;
  screenshots: number;
  first_ts_ms: number;
  last_ts_ms: number;
}

export interface UsageReport {
  schema: number;
  since_ms: number;
  until_ms: number;
  generated_at_ms: number;
  key_count: number;
  totals: UsageRow;
  by_key: UsageRow[];
  by_day: DayRow[];
  ledger_head: { seq: number; hash: string } | null;
}

export interface UsageExport {
  schema: number;
  path: string;
  manifest: string;
  rows: number;
  bytes: number;
  sha256: string;
  truncated: boolean;
}
