import { useId } from "react";
import { Button, Field, SelectInput, TextInput } from "../../../components/ui";

export type ExtractionKind = "text" | "element" | "attribute" | "link" | "image" | "html" | "repeated_record";
export interface SelectorField {
  id: string;
  name: string;
  kind: ExtractionKind;
  selector: string;
  attribute?: string;
  required: boolean;
  repeat: boolean;
  transforms: Array<{ transform: string; value?: string; pattern?: string; replacement?: string }>;
  children: SelectorField[];
}

const kinds: Array<[ExtractionKind, string]> = [
  ["text", "Text"], ["element", "Element"], ["attribute", "Attribute"],
  ["link", "Link URL"], ["image", "Image URL"], ["html", "HTML"], ["repeated_record", "Repeated record"],
];

export default function SelectorEditor({ value, onChange, onTest, busy = false }: {
  value: SelectorField;
  onChange: (next: SelectorField) => void;
  onTest?: () => void;
  busy?: boolean;
}) {
  const base = useId();
  const update = (patch: Partial<SelectorField>) => onChange({ ...value, ...patch });
  return (
    <div className="grid grid-2" aria-label={`Edit selector ${value.name || "field"}`}>
      <Field label="Field name" htmlFor={`${base}-name`}>
        <TextInput id={`${base}-name`} value={value.name} maxLength={128} onChange={(e) => update({ name: e.target.value })} />
      </Field>
      <Field label="Extraction kind" htmlFor={`${base}-kind`}>
        <SelectInput id={`${base}-kind`} value={value.kind} onChange={(e) => update({ kind: e.target.value as ExtractionKind })}>
          {kinds.map(([id, label]) => <option key={id} value={id}>{label}</option>)}
        </SelectInput>
      </Field>
      <Field label="CSS selector" htmlFor={`${base}-selector`} help="Use a CSS selector scoped to the current record container.">
        <TextInput id={`${base}-selector`} value={value.selector} maxLength={4096} spellCheck={false} onChange={(e) => update({ selector: e.target.value })} />
      </Field>
      {value.kind === "attribute" && <Field label="Attribute name" htmlFor={`${base}-attribute`}>
        <TextInput id={`${base}-attribute`} value={value.attribute ?? "href"} maxLength={128} onChange={(e) => update({ attribute: e.target.value })} />
      </Field>}
      <label className="check-row"><input type="checkbox" checked={value.required} onChange={(e) => update({ required: e.target.checked })} /> Required result</label>
      <label className="check-row"><input type="checkbox" checked={value.repeat} onChange={(e) => update({ repeat: e.target.checked })} /> Repeat matches</label>
      <div className="row-actions" style={{ gridColumn: "1 / -1" }}>
        <Button disabled={busy || !value.selector.trim()} onClick={onTest}>Test selector</Button>
        <span className="t-caption t-muted">Transforms: {value.transforms.length ? value.transforms.map((t) => t.transform).join(", ") : "none"}</span>
      </div>
    </div>
  );
}
