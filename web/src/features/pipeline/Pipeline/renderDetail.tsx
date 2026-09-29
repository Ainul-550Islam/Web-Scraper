// renderDetail — split from Pipeline.tsx; barrel re-exports keep every public path & name

import { Fragment } from "react";
import { fmtNum } from "../../../lib/format";
import type { Ctx } from "./Ctx";


export function renderDetail(id: string, c: Ctx) {
  const rowsOf = (pairs: [string, string | number][]) => (
    <dl className="kv" style={{ margin: 0 }}>
      {pairs.map(([k, v]) => (
        <Fragment key={k}>
          <dt>{k}</dt>
          <dd>{v}</dd>
        </Fragment>
      ))}
    </dl>
  );
  switch (id) {
    case "target":
      return (
        <div style={{ display: "grid", gap: 8 }}>
          {(c.pipe ? c.pipe.per_target.map((t) => t.target) : []).map((t) => (
            <code key={t} className="t-code">{t}</code>
          ))}
          {!c.pipe && <p className="t-caption">Run the engine once; target contract comes from GET /pipeline.</p>}
          <p className="t-caption">Per-target start urls, include/exclude patterns, depth and page budgets live in the config — see Configuration Studio.</p>
        </div>
      );
    case "policy":
      return c.policy
        ? rowsOf([
            ["requests / sec", c.policy.rps],
            ["burst", c.policy.burst],
            ["min interval (ms)", c.policy.minInterval],
          ])
        : <p className="t-caption">No default policy attached.</p>;
    case "fetcher":
      return rowsOf([
        ["requests total", fmtNum(c.stages["requests_total"])],
        ["retries", fmtNum(c.stages["requests_retry"])],
        ["body cap", "enforced at max_response_bytes (see config)"],
      ]);
    case "proxy":
      return rowsOf([["egress", fmtNum(c.proxies)], ["rotation", "health-based; credentials stay server-side"]]);
    case "extraction":
      return rowsOf([["raw records", fmtNum(c.stages["records_extracted"])], ["modes", "css (html) + jsonpath (json) selectors per field"]]);
    case "transform":
      return rowsOf([["ops (max per target)", fmtNum(c.stages["transform_ops"])], ["catalog", "23 ops — see Target Builder extraction editor"]]);
    case "schema":
      return rowsOf([["declared fields (max)", fmtNum(c.stages["schema_fields"])], ["drifts", fmtNum(c.stages["schema_drifts"])], ["quarantine", "violations route to quarantine_path (off|coerce|strict)"]]);
    case "dedupe":
      return rowsOf([["duplicates dropped", fmtNum(c.stages["records_deduped"])], ["key", "record fingerprint (content-addressed)"]]);
    case "sink":
      return rowsOf([["backend", c.sinks], ["written", fmtNum(c.stages["records_written"])]], );
    default:
      return null;
  }
}
