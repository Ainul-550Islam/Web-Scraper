// Ctx — split from Pipeline.tsx; barrel re-exports keep every public path & name

import type { PipelineResponse } from "../../../lib/types";

export type Ctx = {
  pipe: PipelineResponse | null;
  stages: Record<string, number>;
  targets: number;
  policy: { rps: number; burst: number; minInterval: number } | null;
  proxies: number;
  sinks: string;
  selected: string | null;
};
