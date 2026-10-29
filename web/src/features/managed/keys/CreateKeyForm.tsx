// CreateKeyForm — the "issue a key" panel.
//
// The only thing that matters here is what gets sent: `scopes` and `limits`
// are the whole of the request body. Nothing is derived client-side that the
// server does not validate again.

import { useState } from "react";
import { Button, Field, NumberInput, TextInput } from "../../../components/ui";
import { Icon } from "../../../components/Icons";
import { createKey, type KeyCreateRequest } from "./KeysClient";
import { SCOPE_ORDER, SCOPE_LABELS, type ScopeName } from "./types";

export interface CreatedKey {
  apiKey: string;
  keyId: string;
}

const DEFAULTS = {
  rps: 5,
  burst: 10,
  maxConcurrent: 2,
  monthlyQuota: 100_000,
  maxBytes: 16 * 1024 * 1024,
};

export function CreateKeyForm({
  tenantId,
  onCreated,
  onError,
}: {
  tenantId: string;
  onCreated: (created: CreatedKey) => void;
  onError: (message: string) => void;
}) {
  const [scopes, setScopes] = useState<ScopeName[]>(["scrape", "jobs_read", "usage_read"]);
  const [rps, setRps] = useState<number | null>(DEFAULTS.rps);
  const [burst, setBurst] = useState<number | null>(DEFAULTS.burst);
  const [maxConcurrent, setMaxConcurrent] = useState<number | null>(DEFAULTS.maxConcurrent);
  const [monthlyQuota, setMonthlyQuota] = useState<number | null>(DEFAULTS.monthlyQuota);
  const [maxBytes, setMaxBytes] = useState<number | null>(DEFAULTS.maxBytes);
  const [domains, setDomains] = useState("");
  const [busy, setBusy] = useState(false);

  const numeric = {
    rps: rps ?? 0,
    burst: burst ?? 0,
    max_concurrent_jobs: maxConcurrent ?? 0,
    monthly_quota: monthlyQuota ?? 0,
    max_response_bytes: maxBytes ?? 0,
  };
  const invalidNumber = Object.values(numeric).some((n) => !Number.isFinite(n) || n <= 0);
  const domainsList = domains
    .split(/[,\s]+/)
    .map((d) => d.trim().toLowerCase())
    .filter(Boolean);

  function toggleScope(scope: ScopeName) {
    setScopes((prev) => (prev.includes(scope) ? prev.filter((s) => s !== scope) : [...prev, scope]));
  }

  async function submit() {
    if (busy) return;
    if (scopes.length === 0) {
      onError("Pick at least one scope — a key with none can authenticate but do nothing.");
      return;
    }
    if (invalidNumber) {
      onError("Every limit must be a positive number.");
      return;
    }
    setBusy(true);
    try {
      const req: KeyCreateRequest = {
        tenant_id: tenantId,
        scopes,
        limits: { ...numeric, allowed_domains: domainsList },
      };
      const issued = await createKey(req);
      onCreated({ apiKey: issued.api_key, keyId: issued.key.id });
      // Only the values that are safe to reset. The scopes stay put so a
      // second key for the same tenant is one click, not a re-entry.
      setDomains("");
      setRps(DEFAULTS.rps);
      setBurst(DEFAULTS.burst);
      setMaxConcurrent(DEFAULTS.maxConcurrent);
      setMonthlyQuota(DEFAULTS.monthlyQuota);
      setMaxBytes(DEFAULTS.maxBytes);
    } catch (e) {
      onError(e instanceof Error ? e.message : "could not create the key");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <Field label="Scopes" help="A key can authenticate without any of these; it just cannot act.">
        <div className="chips" role="group" aria-label="Scopes">
          {SCOPE_ORDER.map((scope) => (
            <label key={scope} className={`chip toggle ${scopes.includes(scope) ? "on" : "off"}`}>
              <input
                type="checkbox"
                checked={scopes.includes(scope)}
                onChange={() => toggleScope(scope)}
              />
              {SCOPE_LABELS[scope] ?? scope}
            </label>
          ))}
        </div>
      </Field>

      <div className="grid grid-2">
        <Field label="Requests / second" help="Token-bucket refill rate.">
          <NumberInput value={rps} onValue={setRps} min={0.1} step={0.5} />
        </Field>
        <Field label="Burst" help="Token-bucket capacity.">
          <NumberInput value={burst} onValue={setBurst} min={1} step={1} />
        </Field>
        <Field label="Max concurrent requests" help="In-flight cap for this key.">
          <NumberInput value={maxConcurrent} onValue={setMaxConcurrent} min={1} step={1} />
        </Field>
        <Field label="Monthly request quota" help="0 means unlimited; the server caps the value.">
          <NumberInput value={monthlyQuota} onValue={setMonthlyQuota} min={0} step={1000} />
        </Field>
        <Field label="Max response bytes" help="Hard cap on what this key may receive.">
          <NumberInput value={maxBytes} onValue={setMaxBytes} min={1024} step={1024} />
        </Field>
        <Field label="Domain allowlist" help="Empty = no restriction beyond the SSRF guard.">
          <TextInput
            value={domains}
            onChange={(e) => setDomains(e.target.value)}
            placeholder="example.com, news.example.com"
          />
        </Field>
      </div>

      {domainsList.length > 0 && (
        <p className="t-caption">
          Allowed: {domainsList.join(", ")} — a key that cannot reach a domain it is not allowed
          for is refused with 403 before any network work.
        </p>
      )}

      <div className="row-end">
        <Button type="submit" variant="primary" disabled={busy || !tenantId}>
          {busy ? "Creating…" : "Create key"}
        </Button>
      </div>
      {!tenantId && (
        <p className="field-error" role="status">
          <Icon.Warn size={12} /> Select a tenant first.
        </p>
      )}
    </form>
  );
}
