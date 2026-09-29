# Export Center

| | |
|---|---|
| Route | `/export` |
| Nav group | Delivery |
| Shortcut | — (none registered) |
| Endpoints | none — file pickers only |
| Polling | none |

Dataset → filter → preview → format → prepare → verify → download. The page is
a **delivery workbench over files you own**: it reads a JSONL/JSON export in-tab,
lets you filter and preview it, composes the exact `harvest export` command for
the formats you chose, and verifies a produced folder's digests in-browser.

## What it does, and what it deliberately does not

| Step | Where it happens |
|---|---|
| read + filter + preview | **browser** (bounded: 20 000 records, 64 columns, 25 preview rows) |
| format selection | **browser**, registry pinned to `harvest::export::ExportFormat` |
| write CSV/JSONL/XLSX/PDF/Parquet/XML/ZIP | **engine host** — `harvest export --format …` |
| manifest + checksums + atomic finalize | **engine host** (see [export formats](../export-formats.md)) |
| verify digests | **browser** (WebCrypto SHA-256) *and* `harvest verify` on the host |

The console never claims to have generated the delivery files. There is no
export endpoint to call — inventing one would mean inventing a server-side job
store the engine does not have. The page prints the command instead:

```
harvest export --from records.jsonl --to ./delivery --format csv,jsonl,xlsx,pdf,parquet,xml,zip --dataset demo-run
```

`--dataset` is sanitized with the engine's key-token rules, so
`--dataset ../../etc/passwd` becomes `dataset-etc_passwd` — no path can escape
the delivery directory.

## Formats

Seven cards, each with extension, purpose, support state and an honest estimate
of what the current selection would produce (rows, sheets, row groups, members).
A format the engine build cannot write renders as **not supported by this engine
build** and is unselectable — the picker never offers a card that would silently
produce nothing.

| Format | Purpose | Output shape |
|---|---|---|
| CSV | spreadsheet-shaped text | header + one row per record |
| JSONL | canonical record stream | one JSON object per line (the manifest's source of truth) |
| XLSX | typed Excel workbook | 100 000 data rows per sheet, header repeated on each |
| PDF | summary report | identity, quality, integrity, ≤ 12 sample rows |
| Parquet | columnar analytics | typed columns, 100 000-row groups |
| XML | system interchange | one `<record>` per record, escaped and deterministic |
| ZIP | one download | selected formats + `manifest.json` + `checksums.txt` + `bundle-manifest.json` |

## Two real, local actions

1. **Prepare filtered JSONL** — writes the current selection back out as
   canonical JSONL with its SHA-256, and offers it as a download. This is the
   input you hand to `harvest export` on the engine host.
2. **Verify a delivery** — drop a produced `manifest.json` plus its payload
   files; each file is re-hashed and compared against the promised digest and
   byte size. States: `match`, `mismatch`, `missing`, `no-digest`.

Verification here is digest-level, and the page says so: record re-counts and the
detached signature belong to `harvest verify` on the machine that holds the
delivery (and the key).

## Honest limits

* No progress percentages. The page's state is one of `idle`, `reading file`,
  `loaded`, `ready`, `source error` — there is no animated bar measuring
  something it cannot measure.
* `crypto.subtle` is unavailable outside secure contexts; when it is missing the
  page disables hashing and preparation rather than pretending to pass.
* Credential-shaped field *names* (`token`, `api_key`, `authorization`, …) are
  displayed as `***` in the preview. Masking happens on screen only — the export
  you prepare carries the real values, because silently altering delivered data
  would be worse than showing it to an operator who already has the file.
* Bounded preview: rows, columns and characters are capped, and the header says
  how much was withheld.

- **Source:** `web/src/pages/ExportCenter.tsx`, `web/src/components/ExportFormatPicker.tsx`, `web/src/components/DataPreviewTable.tsx`
- **Tests:** `web/src/test/export-center.test.tsx` (15 tests: source parsing, unsupported formats, selection, bounded preview, redaction, command composition, prepare/download, verify match + tamper, no-percentage)
- **Engine:** `tests/export_formats.rs` (14 tests: every format written, manifest coverage, tamper detection, bundle members, determinism, dataset-id sanitization)
- **See also:** [delivery](delivery.md) · [docs/export-formats.md](../export-formats.md) · [docs/artifact-integrity.md](../artifact-integrity.md)
