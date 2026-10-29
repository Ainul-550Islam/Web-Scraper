// ApiKeys — the managed API's key table.
//
// Reads `GET /v1/admin/keys`, creates with `POST /v1/admin/keys` and rotates,
// revokes or suspends with the three key-id routes. Every mutation is a real
// round trip; the only thing kept in component state is the plaintext of the
// key that was just issued, and only until the reveal dialog closes.

import { useEffect, useMemo, useState } from "react";
import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtBytes, fmtDateTime } from "../../../lib/format";
import { useToasts } from "../../../hooks/useToasts";
import { Button, Card, EmptyState, ErrorState, PageHeader, SelectInput, Skeleton, StatusChip } from "../../../components/ui";
import { Icon } from "../../../components/Icons";
import { Page } from "../../../components/AppShell";
import { Modal } from "../../../components/Overlays";
import { listKeys, rotateKey, revokeKey, suspendKey } from "./KeysClient";
import { CreateKeyForm } from "./CreateKeyForm";
import { KeyReveal } from "./KeyReveal";
import { KEY_STATUS_LABELS, SCOPE_LABELS, type ApiKey, type TenantList } from "./types";

interface PendingAction {
  key: ApiKey;
  kind: "rotate" | "revoke" | "suspend";
}

const ACTION_LABEL: Record<PendingAction["kind"], string> = {
  rotate: "Rotate",
  revoke: "Revoke",
  suspend: "Suspend",
};

export function ApiKeys() {
  const tenants = usePolled((s) => apiGet<TenantList>("/v1/admin/tenants", { signal: s }), 10_000);
  const [selected, setSelected] = useState<string>("");
  const [keys, setKeys] = useState<ApiKey[]>([]);
  const [listError, setListError] = useState<string | null>(null);
  const [reveal, setReveal] = useState<{ apiKey: string; keyId: string; title: string } | null>(null);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const toasts = useToasts();

  // The tenant list is the only polled resource here; the key list follows it.
  // The effective tenant is *derived*, never written from an effect: the first
  // tenant when nothing is picked, "" when there are none.
  const tenantId = useMemo(() => {
    const list = tenants.data?.tenants ?? [];
    if (list.length === 0) return "";
    return list.some((t) => t.id === selected) ? selected : list[0]!.id;
  }, [tenants.data, selected]);

  useEffect(() => {
    if (!tenantId) return;
    let alive = true;
    listKeys(tenantId)
      .then((out) => {
        if (!alive) return;
        setKeys(out.keys);
        setListError(null);
      })
      .catch((e: unknown) => {
        if (alive) setListError(e instanceof Error ? e.message : "could not read the keys");
      });
    return () => {
      alive = false;
    };
  }, [tenantId, tick]);

  const activeCount = useMemo(() => keys.filter((k) => k.status === "active").length, [keys]);
  const tenantName = tenants.data?.tenants.find((t) => t.id === tenantId)?.name ?? null;

  async function runAction(key: ApiKey, kind: PendingAction["kind"]) {
    setBusy(key.id);
    try {
      if (kind === "rotate") {
        const issued = await rotateKey(key.id);
        setReveal({
          apiKey: issued.api_key,
          keyId: issued.key.id,
          title: `Rotated key for ${issued.key.tenant_id}`,
        });
        toasts.push("ok", "The old plaintext is revoked; store the new one.");
      } else if (kind === "revoke") {
        await revokeKey(key.id);
        toasts.push("ok", `Key ${key.prefix} revoked.`);
      } else {
        await suspendKey(key.id);
        toasts.push("ok", `Key ${key.prefix} suspended.`);
      }
      setPending(null);
      setTick((t) => t + 1);
    } catch (e) {
      toasts.push("err", e instanceof Error ? e.message : `could not ${kind} the key`);
    } finally {
      setBusy(null);
    }
  }

  return (
    <Page>
      <PageHeader
        title="API Keys"
        sub={`${keys.length} key(s) · ${activeCount} active`}
        actions={
          <StatusChip
            status={keys.length === 0 ? "idle" : activeCount === keys.length ? "ok" : "degraded"}
            label={keys.length === 0 ? "no keys yet" : `${activeCount}/${keys.length} active`}
          />
        }
      />

      <Card title="Tenants">
        {tenants.error ? (
          <ErrorState error={tenants.error} />
        ) : tenants.data ? (
          <>
            <div className="row">
              <SelectInput
                aria-label="Tenant"
                value={selected || tenantId}
                onChange={(e) => setSelected(e.target.value)}
              >
                {tenants.data.tenants.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name} ({t.id})
                  </option>
                ))}
              </SelectInput>
              <span className="t-caption">
                {tenants.data.tenant_count} tenant(s) · {tenants.data.key_count} key(s) in total
              </span>
            </div>
          </>
        ) : (
          <div style={{ display: "grid", gap: 8 }}>
            <Skeleton />
            <Skeleton h={10} />
          </div>
        )}
      </Card>

      <div className="grid grid-2">
        <Card title={tenantName ? `Create a key — ${tenantName}` : "Create a key"}>
          <CreateKeyForm
            tenantId={tenantId}
            onError={(message) => toasts.push("err", message)}
            onCreated={(created) => {
              setReveal({
                apiKey: created.apiKey,
                keyId: created.keyId,
                title: `New key for ${tenantName ?? tenantId}`,
              });
              setTick((t) => t + 1);
            }}
          />
        </Card>

        <Card title="Keys" actions={
            <span className="row">
              <Button variant="ghost" className="sm" onClick={() => setTick((t) => t + 1)}>
                Refresh
              </Button>
              <span className="t-caption">{keys.length} listed</span>
            </span>
          }>
          {!tenantId ? (
            <EmptyState icon={<Icon.Api size={16} />} title="No tenant selected" body="Pick a tenant to see its keys." />
          ) : listError ? (
            <div className="state err" role="alert">
              <div className="state-ico"><Icon.Warn size={20} /></div>
              <h3>Could not read the keys</h3>
              <p>{listError}</p>
              <Button onClick={() => setTick((t) => t + 1)}><Icon.Refresh size={13} /> Retry</Button>
            </div>
          ) : keys.length === 0 ? (
            <EmptyState
              icon={<Icon.Api size={16} />}
              title="No keys yet"
              body="Issue one from the form on the left. The plaintext is shown exactly once."
            />
          ) : (
            <table className="data">
              <thead>
                <tr>
                  <th scope="col">Prefix</th>
                  <th scope="col">Status</th>
                  <th scope="col">Scopes</th>
                  <th scope="col">Limits</th>
                  <th scope="col">Expires</th>
                  <th scope="col">Last used</th>
                  <th scope="col">Actions</th>
                </tr>
              </thead>
              <tbody>
                {keys.map((k) => (
                  <tr key={k.id} data-key-id={k.id}>
                    <td>
                      <code className="t-code">{k.prefix}</code>
                      <div className="t-caption">{k.id}</div>
                    </td>
                    <td>
                      <StatusChip status={k.status} label={KEY_STATUS_LABELS[k.status] ?? k.status} />
                    </td>
                    <td>
                      <div className="chips">
                        {k.scopes.map((s) => (
                          <span key={s} className={`chip ${s === "scrape" ? "on" : "off"}`}>
                            {SCOPE_LABELS[s] ?? s}
                          </span>
                        ))}
                      </div>
                    </td>
                    <td>
                      <div className="t-caption">{k.limits.rps} rps · burst {k.limits.burst}</div>
                      <div className="t-caption">
                        concurrency {k.limits.max_concurrent_jobs} · quota{" "}
                        {k.limits.monthly_quota === 0
                          ? "unlimited"
                          : k.limits.monthly_quota.toLocaleString("en-US")}
                      </div>
                      <div className="t-caption">≤ {fmtBytes(k.limits.max_response_bytes)} response</div>
                      {k.limits.allowed_domains.length > 0 && (
                        <div className="t-caption">
                          domains: {k.limits.allowed_domains.join(", ")}
                        </div>
                      )}
                    </td>
                    <td>{k.expires_at ? fmtDateTime(k.expires_at * 1000) : "never"}</td>
                    <td>{k.last_used_at ? fmtDateTime(k.last_used_at * 1000) : "never"}</td>
                    <td>
                      <div className="row">
                        <Button
                          variant="ghost"
                          className="sm"
                          disabled={busy === k.id || k.status !== "active"}
                          onClick={() => setPending({ key: k, kind: "rotate" })}
                        >
                          Rotate
                        </Button>
                        <Button
                          variant="ghost"
                          className="sm"
                          disabled={busy === k.id || k.status !== "active"}
                          onClick={() => setPending({ key: k, kind: "suspend" })}
                        >
                          Suspend
                        </Button>
                        <Button
                          variant="danger"
                          className="sm"
                          disabled={busy === k.id || k.status === "revoked"}
                          onClick={() => setPending({ key: k, kind: "revoke" })}
                        >
                          Revoke
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      </div>

      <p className="t-caption">
        Keys are stored as SHA-256 digests. The hash column is never shown, never copied, and never
        leaves this page — the server holds it, and no response ever echoes a plaintext.
      </p>

      {reveal && (
        <KeyReveal
          key={reveal.apiKey}
          apiKey={reveal.apiKey}
          keyId={reveal.keyId}
          title={reveal.title}
          onClose={() => setReveal(null)}
        />
      )}

      {pending && (
        <Modal
          title={`${ACTION_LABEL[pending.kind]} ${pending.key.prefix}`}
          onClose={() => setPending(null)}
          footer={
            <>
              <Button onClick={() => setPending(null)}>Cancel</Button>
              <Button
                variant={pending.kind === "revoke" ? "danger" : "primary"}
                disabled={busy !== null}
                onClick={() => void runAction(pending.key, pending.kind)}
              >
                {busy === pending.key.id ? "Working…" : `Confirm ${pending.kind}`}
              </Button>
            </>
          }
        >
          {pending.kind === "rotate" ? (
            <p>
              Rotation revokes <code className="t-code">{pending.key.prefix}</code> immediately and
              issues a replacement. The old plaintext stops authenticating on its next request.
            </p>
          ) : pending.kind === "revoke" ? (
            <p>
              Revoking <code className="t-code">{pending.key.prefix}</code> is permanent for this
              key. The record stays on the audit trail; the plaintext is already gone.
            </p>
          ) : (
            <p>
              Suspending <code className="t-code">{pending.key.prefix}</code> stops it from
              authenticating while keeping the key recoverable.
            </p>
          )}
        </Modal>
      )}
    </Page>
  );
}

export default ApiKeys;
