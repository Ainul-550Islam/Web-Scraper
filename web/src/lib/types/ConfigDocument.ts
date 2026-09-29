// ConfigDocument — split from types.ts; barrel re-exports keep every public path & name

import type { TargetCfg } from "./TargetCfg";

export interface ConfigDocument {
  config_version: number;
  name: string;
  user_agent: string;
  respect_robots: boolean;
  concurrency: number;
  request_timeout_secs: number;
  connect_timeout_secs: number;
  max_retries: number;
  retry_base_ms: number;
  max_single_wait_ms: number;
  max_response_bytes: number;
  max_pages: number;
  default_policy: { requests_per_sec: number; burst: number; min_interval_ms: number };
  domain_policies: Record<string, { requests_per_sec: number; burst: number; min_interval_ms: number }>;
  block_backoff_secs: number;
  block_breaker_count: number;
  robots_ttl_secs: number;
  max_inflight_per_proxy: number;
  max_records_per_run: number;
  dlq_path: string | null;
  dlq_max_attempts: number;
  redis_url: string | null;
  ckpt_path: string | null;
  ckpt_every_secs: number;
  ckpt_every_pages: number;
  revalidate_store: string | null;
  revalidate_ttl_secs: number;
  audit_path: string | null;
  events_path: string | null;
  retry_deterministic: boolean;
  scheduling: {
    mode: string;
    max_queue_depth: number;
    per_target_inflight: number;
    worker_restarts: number;
    job_max_age_secs: number;
    retry_budget_per_min: number;
  };
  transport_breaker: {
    min_consecutive: number;
    cooldown_secs: number;
    recovery_successes: number;
  } | null;
  attest_key_path: string | null;
  admin_token: string | null;
  alerts: {
    webhook_url: string | null;
    events: string[];
    min_records: number;
    max_block_rate: number;
    per_target: Record<string, { min_records: number; max_block_rate: number }>;
    quarantine_threshold: number;
    cooldown_secs: number;
    min_severity: string;
  };
  proxies: { id: string; url: string | null; region: string | null }[];
  targets: TargetCfg[];
  output: { kind: string } & Record<string, unknown>;
  metrics_addr: string;
  schedule: { every_secs: number; runs: number; history_path: string };
  /* server-provided extras */
  config_sha256: string;
  validation: "ok" | string;
  _warnings: string[];
  _backends: Record<"sqlite" | "postgres" | "redis", boolean>;
  _production: string[];
  _sources: Record<string, "explicit" | "default" | "env">;
}
