// Control — split from ConfigStudio.tsx; barrel re-exports keep every public path & name

import { getPath, type ControlDef } from "../../../lib/configSchema";
import type { ConfigDocument } from "../../../lib/types";
import { Field, NumberInput, SelectInput, TextInput, Toggle } from "../../../components/ui";


/* ── one control row ─────────────────────────────────────────── */

export function Control({
  def,
  draft,
  server,
  dirty,
  error,
  onChange,
  onReset,
}: {
  def: ControlDef;
  draft: ConfigDocument;
  server: ConfigDocument | null;
  dirty: boolean;
  error: string | null;
  onChange: (v: unknown) => void;
  onReset: () => void;
}) {
  const value = getPath(draft, def.path);
  const srcFlag = (server?._sources?.[def.path.split(".")[0] as string]) ?? null;
  const id = `ctl-${def.path.replaceAll(".", "-")}`;
  const secret = def.secret;

  return (
    <Field
      label={def.label}
      help={def.help}
      note={dirty ? "changed in draft" : srcFlag === "default" ? "default" : srcFlag === "env" ? "${ENV}" : undefined}
      error={error}
      htmlFor={id}
      trailing={
        <>
          {secret && <span className="secret-flag" aria-label="secret field">SECRET</span>}
          <button className="btn ghost sm reset" onClick={onReset} disabled={!dirty} aria-label={`reset ${def.label} to server value`}>
            reset
          </button>
        </>
      }
    >
      {secret ? (
        <TextInput id={id} value="<redacted>" disabled readOnly aria-describedby={error ? `${id}-help` : undefined} />
      ) : def.kind === "toggle" ? (
        <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Toggle checked={Boolean(value)} onChange={onChange} label={def.label} />
          <span className="t-caption">{value ? "on" : "off"}</span>
        </span>
      ) : def.kind === "select" ? (
        <SelectInput id={id} value={String(value ?? "")} onChange={(e) => onChange(e.target.value)} aria-describedby={`${id}-help`}>
          {def.options?.map((o) => <option key={o} value={o}>{o}</option>)}
        </SelectInput>
      ) : def.kind === "number" || def.kind === "float" ? (
        <NumberInput
          id={id}
          unit={def.unit}
          value={typeof value === "number" ? value : value === null ? null : Number(value)}
          onValue={onChange}
          invalid={Boolean(error)}
          aria-describedby={`${id}-help`}
        />
      ) : (
        <TextInput
          id={id}
          value={value === null || value === undefined ? "" : String(value)}
          onChange={(e) => onChange(e.target.value)}
          invalid={Boolean(error)}
          aria-describedby={`${id}-help`}
        />
      )}
    </Field>
  );
}
