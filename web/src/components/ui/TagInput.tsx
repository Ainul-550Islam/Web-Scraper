// TagInput — split from ui.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { Icon } from "../Icons";


export function TagInput({
  values,
  onChange,
  placeholder,
  id,
}: {
  values: string[];
  onChange: (v: string[]) => void;
  placeholder?: string;
  id?: string;
}) {
  const [draft, setDraft] = useState("");
  const commit = () => {
    const v = draft.trim();
    if (v && !values.includes(v)) onChange([...values, v]);
    setDraft("");
  };
  return (
    <div className="tags" id={id}>
      {values.map((v) => (
        <span key={v} className="tag">
          {v}
          <button type="button" aria-label={`remove ${v}`} onClick={() => onChange(values.filter((x) => x !== v))}>
            <Icon.Close size={10} />
          </button>
        </span>
      ))}
      <input
        aria-label="add value"
        value={draft}
        placeholder={placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === ",") {
            e.preventDefault();
            commit();
          } else if (e.key === "Backspace" && !draft && values.length) {
            onChange(values.slice(0, -1));
          }
        }}
        onBlur={commit}
      />
    </div>
  );
}
