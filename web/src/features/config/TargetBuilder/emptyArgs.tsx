// emptyArgs — split from TargetBuilder.tsx; barrel re-exports keep every public path & name

import { TRANSFORM_ARGS, type TransformOp } from "../../../lib/configSchema";


export const emptyArgs = (op: TransformOp): Record<string, unknown> =>
  Object.fromEntries(TRANSFORM_ARGS[op].filter((a) => a.default !== undefined).map((a) => [a.name, a.default]));
