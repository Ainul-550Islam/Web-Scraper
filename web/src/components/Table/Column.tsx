// Column — split from Table.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";

export interface Column<Row> {
  key: string;
  label: string;
  numeric?: boolean;
  hideable?: boolean;
  defaultHidden?: boolean;
  sortValue?: (r: Row) => string | number;
  render: (r: Row) => ReactNode;
}
