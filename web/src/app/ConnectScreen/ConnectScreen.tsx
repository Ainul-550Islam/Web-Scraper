// ConnectScreen — split from ConnectScreen.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { Icon } from "../../components/Icons";
import { Button, Card } from "../../components/ui";
import { OPTS, auth, probeHealth, setApiBase, type ApiError, type ProbeResult } from "../../lib/api";
import { MOCK_ENABLED } from "../../lib/mock";
import { CAPABILITIES } from "./CAPABILITIES";


export function ConnectScreen({ onConnected }: { onConnected: () => void }) {
  const [base, setBase] = useState(OPTS.BASE);
  const [token, setToken] = useState("");
  const [probing, setProbing] = useState(false);
  const [result, setResult] = useState<ProbeResult | null>(null);

  async function connect() {
    setProbing(true);
    setResult(null);
    const probe = await probeHealth(base.trim() || OPTS.BASE, token.trim() || null);
    setResult(probe);
    setProbing(false);
    if (probe.ok) {
      // adopt the working endpoint + keep the token in memory only, then
      // let the shell's poller take over from here
      setApiBase(base.trim() || OPTS.BASE);
      auth.set(token.trim() || null);
      onConnected();
    }
  }

  const errHint = (e: ApiError | null): string =>
    e?.kind === "unauthorized"
      ? "reachable, but a bearer token is required for this bind"
      : e?.kind === "timeout"
        ? "no answer within 4 s — is the engine actually running?"
        : e?.kind === "notfound"
          ? "answering, but /health is missing — is this the harvest admin API?"
          : "could not reach that address";

  return (
    <div className="connect-wrap">
      <div className="connect-card">
        <div className="connect-brand">
          <span className="brand-mark" aria-hidden="true">
            <Icon.Overview size={20} />
          </span>
          <div>
            <h1 className="t-display">Harvest Console</h1>
            <p className="t-caption">
              operator surface for the harvest Rust engine
              {MOCK_ENABLED ? " — demo mode active, no connection needed" : ""}
            </p>
          </div>
        </div>

        <ul className="connect-caps" aria-label="what the console does">
          {CAPABILITIES.map((c) => (
            <li key={c}>{c}</li>
          ))}
        </ul>

        {!MOCK_ENABLED && (
          <Card>
            <form
              className="connect-form"
              onSubmit={(e) => {
                e.preventDefault();
                void connect();
              }}
            >
              <label className="field">
                <span>API endpoint</span>
                <input
                  className="input"
                  type="text"
                  spellCheck={false}
                  aria-label="API endpoint"
                  value={base}
                  onChange={(e) => setBase(e.target.value)}
                  placeholder="/api or http://127.0.0.1:9678"
                />
                <span className="t-caption">
                  dev uses the vite proxy (`/api`); point it straight at the
                  admin address when serving the built console elsewhere
                </span>
              </label>
              <label className="field">
                <span>Bearer token (only if the API binds non-loopback)</span>
                <input
                  className="input"
                  type="password"
                  aria-label="Bearer token"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  autoComplete="off"
                  placeholder="kept in memory for this tab only"
                />
              </label>
              <div className="connect-actions">
                <Button type="submit" variant="primary" disabled={probing}>
                  {probing ? <><Icon.Refresh size={13} className="spin" /> Probing…</> : <><Icon.Overview size={13} /> Connect & probe health</>}
                </Button>
                <Button variant="ghost" onClick={onConnected}>
                  Open console anyway
                </Button>
              </div>

              <div role="status" aria-live="polite" className="connect-status">
                {result === null && !probing && (
                  <span className="t-caption">
                    not connected · start the engine with
                    <code className="t-code"> harvest serve --config demo/localhost.json</code>
                  </span>
                )}
                {result?.ok && (
                  <span className="cs ok">
                    <i className="dot ok" aria-hidden="true" /> health ok
                    {result.latencyMs !== null ? ` · ${result.latencyMs} ms rtt` : ""}
                    {" · "}engine version is not exposed by this API, so none is shown
                  </span>
                )}
                {result && !result.ok && (
                  <span className="cs err">
                    <i className="dot err" aria-hidden="true" /> {errHint(result.error)}
                    {result.error?.status !== null && result.error?.status !== undefined ? (
                      <> <code className="t-code">HTTP {result.error.status}</code></>
                    ) : null}
                    {result.error?.requestId ? (
                      <> <code className="t-code">{result.error.requestId}</code></>
                    ) : null}
                  </span>
                )}
              </div>
            </form>
          </Card>
        )}

        <p className="t-caption connect-foot">
          The token field is optional, session-only, and never written to
          storage; the endpoint you connect to is remembered as a plain
          preference. All console reads are safe to retry — the API is
          loopback-first and bearer-gated when exposed.
        </p>
      </div>
    </div>
  );
}
