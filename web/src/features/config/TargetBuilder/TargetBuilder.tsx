// TargetBuilder — split from TargetBuilder.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import type { TargetCfg } from "../../../lib/types";
import { Button, Card, CopyButton, Field, NumberInput, SelectInput, TagInput, TextInput, Toggle } from "../../../components/ui";
import { Drawer } from "../../../components/Overlays";
import { PipelineConnector, PipelineNode } from "../../../components/patterns";
import { Icon } from "../../../components/Icons";
import type { TransformEntry } from "./TransformEntry";
import { TransformList } from "./TransformList";
import { TagInputLike } from "./TagInputLike";

export function TargetBuilder({
  target,
  onChange,
  onClose,
}: {
  target: TargetCfg;
  onChange: (t: TargetCfg) => void;
  onClose: () => void;
}) {
  const [fieldSel, setFieldSel] = useState<string | null>(Object.keys(target.extract)[0] ?? null);
  const set = <K extends keyof TargetCfg>(k: K, v: TargetCfg[K]) => onChange({ ...target, [k]: v });
  const fields = Object.keys(target.extract);
  const field = fieldSel ? target.extract[fieldSel] : null;

  const errStart = target.start_url && !/^https?:\/\//.test(target.start_url) ? "must start with http(s)://" : null;
  const errId = !target.id ? "required" : /[\s/]/.test(target.id) ? "no spaces or slashes" : null;
  const errSchemaType = Object.entries(target.schema.fields).find(([, v]) => !["string", "number", "bool", "array"].includes(v));

  return (
    <Drawer
      title={
        <span style={{ display: "flex", gap: 10, alignItems: "center" }}>
          <Icon.Config size={14} /> Target Builder
          <code className="t-code">{target.id || "…"}</code>
        </span>
      }
      onClose={onClose}
    >
      <div className="pipe" role="img" aria-label="actual processing chain for this target">
        {([
          ["Target", `${fields.length} extract fields`],
          ["Fetch", `mode ${target.mode} · depth ${target.max_depth}`],
          ["Extract", target.mode === "json" ? "jsonpath selectors" : "css selectors"],
          ["Transform", `${Object.values(target.extract).reduce((a, f) => a + f.transforms.length, 0)} ops`],
          ["Validate", `schema ${target.schema.enforce}`],
          ["Dedupe", "fingerprint"],
          ["Sink", "config output"],
        ] as [string, string][]).map(([t, d], i, arr) => (
          <span key={t} style={{ display: "contents" }}>
            <PipelineNode as="div" title={t} meta={d} />
            {i < arr.length - 1 && <PipelineConnector />}
          </span>
        ))}
      </div>

      <Card title={<><span className="chip idle">1</span> Identity</>}>
        <div className="ctrl-grid">
          <Field label="Target id" error={errId} help="Slug — feeds filesystem paths, so dots/slashes are rejected.">
            <TextInput value={target.id} invalid={!!errId} onChange={(e) => set("id", e.target.value)} />
          </Field>
          <Field label="Start URL" error={errStart} help="Absolute http(s) URL. Sitemap may extend the seed set.">
            <TextInput value={target.start_url} invalid={!!errStart} onChange={(e) => set("start_url", e.target.value)} />
          </Field>
        </div>
      </Card>

      <Card title={<><span className="chip idle">2</span> Crawl</>}>
        <div className="ctrl-grid">
          <Field label="Mode" help="html follows links & css extraction; json extracts via jsonpath.">
            <SelectInput value={target.mode} onChange={(e) => set("mode", e.target.value)}>
              <option value="html">html</option>
              <option value="json">json</option>
            </SelectInput>
          </Field>
          <Field label="Max depth" help="Hops from the seed before links stop being followed.">
            <NumberInput value={target.max_depth} min={0} unit="" onValue={(v) => set("max_depth", v ?? 0)} />
          </Field>
          <Field label="Max pages" help="Per-target page budget for the run.">
            <NumberInput value={target.max_pages} min={1} onValue={(v) => set("max_pages", v ?? 1)} />
          </Field>
          <Field label="Follow links" help="Off = single-page fetch of the seed.">
            <Toggle checked={target.follow_links} onChange={(v) => set("follow_links", v)} label="follow links" />
          </Field>
          <Field label="Link selector" help="CSS scoping the link crawler (empty = all anchors, engine-absolutized).">
            <TextInput value={target.link_selector ?? ""} placeholder="a.product" onChange={(e) => set("link_selector", e.target.value || null)} />
          </Field>
          <Field label="Sitemap" help="Discover URLs via the host's sitemap.xml at crawl start.">
            <Toggle checked={target.sitemap} onChange={(v) => set("sitemap", v)} label="sitemap discovery" />
          </Field>
          {target.sitemap && (
            <Field label="Sitemap URL" help="Override the auto-discovered location.">
              <TextInput value={target.sitemap_url ?? ""} onChange={(e) => set("sitemap_url", e.target.value || null)} />
            </Field>
          )}
          <Field label="Include patterns" help="Substring allow-list applied to candidate URLs.">
            <TagInput values={target.include} onChange={(v) => set("include", v)} placeholder="/product/" />
          </Field>
          <Field label="Exclude patterns" help="Substring deny-list — wins over include.">
            <TagInput values={target.exclude} onChange={(v) => set("exclude", v)} placeholder="/private/" />
          </Field>
        </div>
      </Card>

      <Card
        title={<><span className="chip idle">3</span> Extraction</>}
        actions={<span className="t-caption">{fields.length} field(s)</span>}
      >
        <div style={{ display: "grid", gap: 10 }}>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }} role="listbox" aria-label="fields">
            {fields.map((f) => (
              <button key={f} type="button" className={`btn sm ${f === fieldSel ? "primary" : ""}`} aria-selected={f === fieldSel} role="option" onClick={() => setFieldSel(f)}>
                {f} <span className="t-caption">· {target.extract[f]!.transforms.length ? `${target.extract[f]!.transforms.length}t` : "raw"}</span>
              </button>
            ))}
            <button
              type="button"
              className="btn sm"
              onClick={() => {
                const name = window.prompt("New field name")?.trim();
                if (name && !target.extract[name])
                  onChange({ ...target, extract: { ...target.extract, [name]: { css: "", path: null, attr: null, all: false, transforms: [] } } });
              }}
            >
              + field
            </button>
          </div>
          {field && fieldSel && (
            <div className="ctrl-grid">
              <Field label="Field name">
                <TextInput
                  value={fieldSel}
                  onChange={(e) => {
                    const next = e.target.value;
                    if (!next || (next !== fieldSel && target.extract[next])) return;
                    const rest = { ...target.extract };
                    delete rest[fieldSel];
                    onChange({ ...target, extract: { ...rest, [next]: field } });
                    setFieldSel(next);
                  }}
                />
              </Field>
              <Field label="CSS selector" help="html mode — element/text extractor.">
                <TextInput value={field.css} placeholder="h1.title" onChange={(e) => setField(fieldSel, { css: e.target.value })} />
              </Field>
              <Field label="JSON path" help="json mode — e.g. data.items[*].name.">
                <TextInput value={field.path ?? ""} placeholder="data.items[*].name" onChange={(e) => setField(fieldSel, { path: e.target.value || null })} />
              </Field>
              <Field label="Attribute" help="Extract this attribute instead of text (e.g. href).">
                <TextInput value={field.attr ?? ""} placeholder="href" onChange={(e) => setField(fieldSel, { attr: e.target.value || null })} />
              </Field>
              <Field label="All matches" help="Collect every match into an array rather than the first.">
                <Toggle checked={field.all} onChange={(v) => setField(fieldSel, { all: v })} label="all matches" />
              </Field>
              <div style={{ alignSelf: "end" }}>
                <Button
                  variant="danger"
                  onClick={() => {
                    const rest = { ...target.extract };
                    delete rest[fieldSel];
                    onChange({ ...target, extract: rest });
                    setFieldSel(Object.keys(rest)[0] ?? null);
                  }}
                >
                  Remove field
                </Button>
              </div>
            </div>
          )}
          {field && fieldSel && (
            <TransformList
              ops={field.transforms as TransformEntry[]}
              onChange={(list) => setField(fieldSel, { transforms: list })}
            />
          )}
        </div>
      </Card>

      <Card title={<><span className="chip idle">4</span> Schema contract</>}>
        <div className="ctrl-grid">
          <Field label="Enforcement" help="off = store as extracted · coerce = best-effort typing · strict = violations quarantine.">
            <SelectInput value={target.schema.enforce} onChange={(e) => set("schema", { ...target.schema, enforce: e.target.value as TargetCfg["schema"]["enforce"] })}>
              <option value="off">off</option>
              <option value="coerce">coerce</option>
              <option value="strict">strict</option>
            </SelectInput>
          </Field>
          <Field label="Quarantine path" help="strict mode writes violations here (with reason codes).">
            <TextInput
              value={target.schema.quarantine_path ?? ""}
              onChange={(e) => set("schema", { ...target.schema, quarantine_path: e.target.value || null })}
            />
          </Field>
          <Field
            label="Declared types"
            error={errSchemaType ? `"${errSchemaType[1]}" invalid for ${errSchemaType[0]} — string | number | bool | array` : null}
            help="One field = type per line, e.g. price = number"
          >
            <TagInputLike
              value={target.schema.fields}
              onChange={(fields) => set("schema", { ...target.schema, fields })}
            />
          </Field>
        </div>
      </Card>
      <Card
        title={<><span className="chip info">live</span> Generated target config</>}
        actions={<CopyButton text={JSON.stringify(target, null, 2)} label="Copy target JSON" />}
      >
        <pre className="t-code" style={{ margin: 0, whiteSpace: "pre-wrap", fontSize: 12 }}>
          {JSON.stringify(target, null, 2)}
        </pre>
        <p className="t-caption" style={{ margin: "6px 0 0" }}>
          Stages 5–7 (transform, schema, dedupe/sink) are represented by the fields above and the
          global output lane — the builder never emits a field the engine would not read.
        </p>
      </Card>
    </Drawer>
  );

  function setField(name: string, patch: Partial<TargetCfg["extract"][string]>) {
    onChange({ ...target, extract: { ...target.extract, [name]: { ...target.extract[name]!, ...patch } } });
  }
}
