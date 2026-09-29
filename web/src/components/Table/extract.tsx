// extract — split from Table.tsx; barrel re-exports keep every public path & name

import type { Column } from "./Column";


export function extract<Row>(r: Row, col: Column<Row>): string {
  const v = (r as Record<string, unknown>)[col.key];
  return typeof v === "number" ? String(v) : String(v ?? "");
}
