// InlineError — split from ui.tsx; barrel re-exports keep every public path & name

import { Icon } from "../Icons";
import type { ApiError } from "../../lib/api";


/** Renders an error compactly for table footers. Never shows stacks. */
export function InlineError({ error }: { error: ApiError | null }) {
  if (!error) return null;
  return (
    <div className="field-error" role="status" style={{ padding: "8px 16px" }}>
      <Icon.Warn size={12} /> {error.message}
      {error.requestId ? ` · ${error.requestId}` : ""}
    </div>
  );
}
