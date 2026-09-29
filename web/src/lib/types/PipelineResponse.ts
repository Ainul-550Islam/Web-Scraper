// PipelineResponse — split from types.ts; barrel re-exports keep every public path & name

import type { PipelineTarget } from "./PipelineTarget";

export interface PipelineResponse {
  per_target: PipelineTarget[];
  pipeline_failures: number;
  quarantined: number;
  schema_drifts: number;
  written: number;
}
