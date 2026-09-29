// crossFieldErrors — split from configSchema.ts; barrel re-exports keep every public path & name

import type { ConfigDocument } from "../types";
import { getPath } from "./getPath";


/** Cross-field rule lifted directly from Rust validate(): honest-UA policy. */
export function crossFieldErrors(doc: ConfigDocument): string[] {
  const errs: string[] = [];
  if (doc.respect_robots && /mozilla/i.test(doc.user_agent)) {
    errs.push(
      'user_agent must not contain "mozilla" while respect_robots is true (honest bot identity)',
    );
  }
  if (doc.concurrency < 1) errs.push("concurrency must be ≥ 1");
  const kind = doc.output?.kind;
  if ((kind === "jsonl" || kind === "csv") && !getPath(doc, "output.path")) {
    errs.push(`output.kind "${kind}" needs a non-empty path`);
  }
  return errs;
}
