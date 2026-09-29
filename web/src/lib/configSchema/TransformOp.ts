// TransformOp — split from configSchema.ts; barrel re-exports keep every public path & name

import type { TRANSFORM_OPS } from "./TRANSFORM_OPS";

export type TransformOp = (typeof TRANSFORM_OPS)[number];
