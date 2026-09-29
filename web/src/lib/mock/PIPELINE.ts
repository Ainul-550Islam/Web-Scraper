// PIPELINE — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";


export const PIPELINE: T.PipelineResponse = {
  per_target: [
    { target: "demo-catalog", enforce: "off", schema_fields: 4, transform_ops: 0 },
    { target: "demo-news", enforce: "off", schema_fields: 0, transform_ops: 0 },
  ],
  pipeline_failures: 0, quarantined: 0, schema_drifts: 0, written: 5,
};
