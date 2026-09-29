// StatusBar — split from AppShell.tsx; barrel re-exports keep every public path & name

import { type RouteState } from "../../app/router";
import { ROUTE_BY_PATH } from "../../app/routes";
import { useConnection } from "../../lib/connection";
import { fmtClock } from "../../lib/format";
import { STATE_LABEL } from "./STATE_LABEL";


export function StatusBar({ route }: { route: RouteState }) {
  const conn = useConnection();
  const def = ROUTE_BY_PATH.get(route.path);
  return (
    <footer className="statusbar" aria-label="Connection status">
      <span className="statusbar-item">
        <i
          className={`dot ${
            conn.state === "offline" ? "err" : conn.state === "paused" ? "warn" : conn.state === "live" ? "ok live" : "ok"
          }`}
          aria-hidden="true"
        />
        {STATE_LABEL[conn.state]}
      </span>
      <span className="statusbar-item">API {conn.base || "same-origin"}</span>
      <span className="statusbar-item">
        engine {conn.health ? (conn.health.running ? "running" : "idle") : "unreachable"}
      </span>
      {conn.latencyMs !== null && (
        <span className="statusbar-item">rtt {conn.latencyMs} ms</span>
      )}
      {conn.updatedAt && (
        <span className="statusbar-item">updated {fmtClock(Math.floor(conn.updatedAt / 1000))} · {conn.intervalSecs}s cadence</span>
      )}
      <span className="statusbar-item" style={{ marginLeft: "auto" }}>{def?.title}</span>
    </footer>
  );
}
