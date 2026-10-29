// KeysClient — the typed operator client for `/v1/admin/*`.
//
// One function per endpoint, one path per function, so a page never builds a
// URL by hand. The console bearer is attached by `lib/api`'s `headers()`; the
// customer endpoints under `/v1/*` are deliberately NOT reachable from here.

import { apiGet, apiPost } from "../../../lib/api";
import type {
  ApiKey,
  KeyIssued,
  KeyList,
  TenantList,
  UsageExport,
  UsageReport,
} from "./types";

export const KEYS_PATH = "/v1/admin/keys";
export const TENANTS_PATH = "/v1/admin/tenants";
export const USAGE_PATH = "/v1/admin/usage";

export interface KeyCreateRequest {
  tenant_id: string;
  scopes?: string[];
  limits?: Partial<{
    rps: number;
    burst: number;
    max_concurrent_jobs: number;
    monthly_quota: number;
    max_response_bytes: number;
    allowed_domains: string[];
  }>;
}

export async function listTenants(): Promise<TenantList> {
  return apiGet<TenantList>(TENANTS_PATH, { noCache: true });
}

export async function createTenant(name: string): Promise<TenantList> {
  // The tenant is created and then listed in one round trip: the create
  // response carries the record, and the page needs the full list anyway.
  await apiPost<{ schema: number; tenant: unknown }>(TENANTS_PATH, { name });
  return listTenants();
}

export async function listKeys(tenantId: string): Promise<KeyList> {
  return apiGet<KeyList>(KEYS_PATH, { params: { tenant_id: tenantId }, noCache: true });
}

export async function createKey(req: KeyCreateRequest): Promise<KeyIssued> {
  return apiPost<KeyIssued>(KEYS_PATH, req);
}

/** Rotation revokes the old plaintext server-side; the new one is returned
 *  exactly once, here. */
export async function rotateKey(keyId: string): Promise<KeyIssued> {
  return apiPost<KeyIssued>(`/v1/admin/keys/${encodeURIComponent(keyId)}/rotate`, {});
}

export async function revokeKey(keyId: string): Promise<ApiKey> {
  const out = await apiPost<{ schema: number; key: ApiKey }>(
    `/v1/admin/keys/${encodeURIComponent(keyId)}/revoke`,
    {},
  );
  return out.key;
}

export async function suspendKey(keyId: string): Promise<ApiKey> {
  const out = await apiPost<{ schema: number; key: ApiKey }>(
    `/v1/admin/keys/${encodeURIComponent(keyId)}/suspend`,
    {},
  );
  return out.key;
}

/** `since`/`until` are epoch milliseconds on the wire; the page sends the
 *  calendar bounds it collected and lets the server do the arithmetic. */
export interface UsageWindow {
  keyId?: string;
  sinceMs?: number;
  untilMs?: number;
  limit?: number;
}

export async function fetchUsage(
  window: UsageWindow,
  signal?: AbortSignal,
): Promise<UsageReport> {
  return apiGet<UsageReport>(USAGE_PATH, {
    params: {
      key_id: window.keyId,
      since: window.sinceMs,
      until: window.untilMs,
      limit: window.limit,
    },
    noCache: true,
    signal,
  });
}

/** The export route is a POST but takes its whole filter as query params —
 *  a body there would be silently ignored. */
export async function exportUsage(
  window: UsageWindow & { format?: "jsonl" | "csv" },
): Promise<UsageExport> {
  const qs = new URLSearchParams();
  if (window.keyId) qs.set("key_id", window.keyId);
  if (window.sinceMs) qs.set("since", String(window.sinceMs));
  if (window.untilMs) qs.set("until", String(window.untilMs));
  qs.set("format", window.format ?? "jsonl");
  return apiPost<UsageExport>(`/v1/admin/usage/export?${qs.toString()}`, {});
}
