// TableProps — split from Table.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";
import type { ApiError } from "../../lib/api";
import type { Column } from "./Column";


export interface TableProps<Row> {
  caption: string;
  columns: Column<Row>[];
  rows: Row[];
  rowKey: (r: Row) => string;
  total?: number; // server-side paging total (undefined => no pager)
  limit?: number;
  offset?: number;
  onPage?: (offset: number) => void;
  loading?: boolean;
  error?: ApiError | null;
  onRowClick?: (r: Row) => void;
  selectedKey?: string | null;
  emptyTitle: string;
  emptyBody?: ReactNode;
}
