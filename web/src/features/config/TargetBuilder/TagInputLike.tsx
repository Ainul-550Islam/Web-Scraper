// TagInputLike — split from TargetBuilder.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { Button, SelectInput, TextInput } from "../../../components/ui";
import { Icon } from "../../../components/Icons";


/** field = type rows editor (keys kept in insertion order → object) */
export function TagInputLike({ value, onChange }: { value: Record<string, string>; onChange: (v: Record<string, string>) => void }) {
  const [name, setName] = useState("");
  const [type, setType] = useState("string");
  const entries = Object.entries(value);
  return (
    <div style={{ display: "grid", gap: 6 }}>
      {entries.length === 0 && <span className="t-caption">No declared fields — schema is shape-only off.</span>}
      {entries.map(([k, v]) => (
        <span key={k} className="tag" style={{ alignSelf: "flex-start", width: "100%", maxWidth: "100%", justifyContent: "space-between" }}>
          <code>{k} = {v}</code>
          <button type="button" aria-label={`remove ${k}`} onClick={() => {
            const rest = { ...value };
            delete rest[k];
            onChange(rest);
          }}><Icon.Close size={10} /></button>
        </span>
      ))}
      <span style={{ display: "flex", gap: 6 }}>
        <TextInput placeholder="field" value={name} onChange={(e) => setName(e.target.value)} style={{ flex: 1 }} />
        <SelectInput value={type} onChange={(e) => setType(e.target.value)} style={{ width: 110 }} aria-label="declared type">
          <option>string</option>
          <option>number</option>
          <option>bool</option>
          <option>array</option>
        </SelectInput>
        <Button
          onClick={() => {
            const k = name.trim();
            if (!k) return;
            onChange({ ...value, [k]: type });
            setName("");
          }}
        >
          set
        </Button>
      </span>
    </div>
  );
}
