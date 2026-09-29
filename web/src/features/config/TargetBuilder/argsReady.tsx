// argsReady — split from TargetBuilder.tsx; barrel re-exports keep every public path & name

import { TRANSFORM_ARGS, type TransformOp } from "../../../lib/configSchema";


/** true when every required arg of `op` has a usable value in `args` */
export function argsReady(op: TransformOp, args: Record<string, unknown>): boolean {
  return TRANSFORM_ARGS[op].filter((a) => a.required).every((a) => {
    const v = args[a.name];
    return a.kind === "list" ? Array.isArray(v) && v.length > 0 : v !== undefined && v !== null && v !== "";
  });
}
