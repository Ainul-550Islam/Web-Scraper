// STAGES — split from Pipeline.tsx; barrel re-exports keep every public path & name

import { fmtNum } from "../../../lib/format";
import type { Stage } from "./Stage";


export const STAGES: Stage[] = [
  { id: "target", title: "Target", source: "GET /config → targets",
    meta: (c) => [`${fmtNum(c.targets)} targets`, c.selected ? "selected target in panel" : "start urls, include/exclude, budgets"] },
  { id: "policy", title: "Policy", source: "GET /config → default_policy",
    meta: (c) => (c.policy ? [`rate ${c.policy.rps} rps`, `burst ${c.policy.burst}`, `min interval ${c.policy.minInterval} ms`] : ["no policy attached"]) },
  { id: "fetcher", title: "Fetcher", source: "GET /stats → metrics",
    meta: (c) => [`${fmtNum(c.stages["requests_total"] ?? 0)} requests`, `${fmtNum(c.stages["requests_retry"] ?? 0)} retries`] },
  { id: "proxy", title: "Proxy", source: "GET /stats → proxies",
    meta: (c) => [`${fmtNum(c.proxies)} egress(es)`, "health-weighted rotation"] },
  { id: "extraction", title: "Extraction", source: "GET /pipeline → per_target",
    meta: (c) => [`${fmtNum(c.stages["records_extracted"] ?? 0)} raw records`] },
  { id: "transform", title: "Transform", source: "GET /pipeline → transform_ops",
    meta: (c) => [`${fmtNum(c.stages["transform_ops"] ?? 0)} ops (max per target)`] },
  { id: "schema", title: "Schema", source: "GET /pipeline → enforce/fields",
    meta: (c) => [`${fmtNum(c.stages["schema_fields"] ?? 0)} declared fields`, `${fmtNum(c.stages["schema_drifts"] ?? 0)} drifts`] },
  { id: "dedupe", title: "Dedupe", source: "GET /stats → metrics",
    meta: (c) => [`${fmtNum(c.stages["records_deduped"] ?? 0)} duplicates dropped`] },
  { id: "sink", title: "Sink", source: "GET /config → output",
    meta: (c) => [c.sinks, `${fmtNum(c.stages["records_written"] ?? 0)} written`] },
];
