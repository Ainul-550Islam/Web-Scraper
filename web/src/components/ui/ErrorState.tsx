// ErrorState — split from ui.tsx; barrel re-exports keep every public path & name

import { Icon } from "../Icons";
import { ApiError } from "../../lib/api";
import { Button } from "./Button";


export function ErrorState({
  error,
  onRetry,
}: {
  error: ApiError | Error | null;
  onRetry?: () => void;
}) {
  const ae = error instanceof ApiError ? error : null;
  const msg =
    ae?.kind === "unauthorized"
      ? "The admin API requires a bearer token. Set it in the console header — it stays in memory only."
      : ae?.kind === "slowdown"
        ? "The admin API is rate limiting the console (429) — polling backs off automatically; no action needed."
        : ae?.kind === "timeout"
          ? "The admin API did not answer in time. It may be busy or wedged; retrying is safe — all console reads are idempotent."
          : ae?.message ?? "The request failed.";
  const heading =
    ae?.kind === "network"
      ? "Cannot reach the admin API"
      : ae?.kind === "timeout"
        ? "Request timed out"
        : "Request failed";
  return (
    <div className="state err" role="alert">
      <div className="state-ico"><Icon.Warn size={20} /></div>
      <h3>{heading}</h3>
      <p>
        {msg}
        {ae?.status !== null && ae?.status !== undefined && (
          <> <code className="t-code">HTTP {ae.status}</code></>
        )}
        {ae?.requestId && (
          <> <code className="t-code">{ae.requestId}</code></>
        )}
      </p>
      {onRetry && <Button onClick={onRetry}><Icon.Refresh size={13} /> Retry</Button>}
    </div>
  );
}
