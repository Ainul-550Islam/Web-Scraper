import { Button, EmptyState } from "../../components/ui";

type Row = Record<string, unknown>;
function display(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (Array.isArray(value)) return `${value.length} items`;
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}
export default function PreviewTable({ records, onRefresh, busy = false, error = null }: {
  records: Row[];
  onRefresh?: () => void;
  busy?: boolean;
  error?: string | null;
}) {
  const columns = [...new Set(records.flatMap((record) => Object.keys(record)))].slice(0, 32);
  return <section aria-label="Selector result preview">
    <div className="row-actions"><span className="t-caption t-muted">{records.length} sample record{records.length === 1 ? "" : "s"}</span>{onRefresh && <Button disabled={busy} onClick={onRefresh}>{busy ? "Testing…" : "Refresh preview"}</Button>}</div>
    {error ? <div className="field-error" role="alert">{error}</div> : records.length === 0 ? <EmptyState title="No records yet" body="Test the selector tree against the current browser page." /> : (
      <div className="table-wrap"><table className="data"><thead><tr>{columns.map((key) => <th key={key}>{key}</th>)}</tr></thead><tbody>{records.slice(0, 100).map((record, i) => <tr key={String(record.id ?? i)}>{columns.map((key) => <td key={key} title={display(record[key])}><span className="preview-cell">{display(record[key])}</span></td>)}</tr>)}</tbody></table></div>
    )}
  </section>;
}
