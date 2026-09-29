# Export formats

`harvest export` turns the engine's JSONL output into a delivery folder. CSV and
JSONL are the historical pair; XLSX, PDF, Parquet, XML and the ZIP bundle are
adapters over the **same record set**, written in the same run, covered by the
**same manifest**. There is no second export pipeline.

```
harvest export --from out/records.jsonl --to ./delivery \
  --format csv,jsonl,xlsx,pdf,parquet,xml,zip --dataset demo-run
```

`--format` is repeatable and comma-separated; the default is `csv,jsonl`, so
existing scripts are unchanged. `--dataset` names the newer artifacts
(`dataset-<id>.<ext>`, sanitized with the engine's key-token rules); without it
they follow the historical `records.<ext>` / `<target>.<ext>` naming.

## What each format is

| Format | File | Content | Read with |
|---|---|---|---|
| CSV | `records.csv` | header + one row per record, objects dot-flattened | any spreadsheet |
| JSONL | `records.jsonl` | canonical record stream (meta + `fields`) | `jq`, pandas |
| XLSX | `dataset-<id>.xlsx` | typed workbook: strings, ints, floats, booleans, ISO dates, blanks for nulls | Excel, LibreOffice, openpyxl |
| PDF | `dataset-<id>.pdf` | summary report: identity, targets, quality, integrity, bounded sample | any PDF reader |
| Parquet | `dataset-<id>.parquet` | columnar file, PLAIN encoding, uncompressed, UTF8 strings | pandas, DuckDB, Spark, pyarrow |
| XML | `dataset-<id>.xml` | escaped, deterministic elements with `type` and `null` attributes | any XML parser |
| ZIP | `dataset-<id>.zip` | selected formats + `manifest.json` + `checksums.txt` + `bundle-manifest.json` | `unzip` |

## Guarantees that hold for every format

* **Bounded.** Row, column and string caps are enforced, not suggested. XLSX
  rolls into additional sheets (`records`, `records-2`, …) above 100 000 rows;
  Parquet chunks into 100 000-row groups; the PDF renders a bounded sample
  (12 rows by default) because a report is not a data dump.
* **Deterministic.** Same records ⇒ same bytes for CSV, JSONL, XLSX, Parquet and
  XML. The PDF carries the wall-clock generation time and is therefore
  reproducible *given* that timestamp, like the manifest.
* **Atomic.** Every file is written to a staging path, flushed and renamed. An
  interrupted export leaves no manifest — never a partial one that looks finished.
* **Covered.** Every payload file is listed in `manifest.json` with its SHA-256,
  byte count and line count, and in `checksums.txt` (`sha256sum -c` compatible).

## XLSX

OOXML SpreadsheetML written directly: inline strings, numeric cells, `t="b"`
booleans, date serials with a `yyyy-mm-dd` style, and blank cells for nulls
(never the text "null"). No shared-string table, which keeps the writer stateless
across sheets. Excel's own limits are respected: 1 048 576 rows, 16 384 columns
and 32 767 characters per cell; our defaults sit well inside them.

## PDF

A report, not a dump: delivery identity, run list, per-target counts, data
quality, integrity (per-file digests) and a bounded sample table. Counters that
the export input does not carry — quarantined records, schema drift, retries,
robots denials — come from the run history when `--history` is supplied, and the
report says they are unavailable when it is not. It never invents them.

## Parquet

Parquet 1.0 with `PAR1` magic, data pages in PLAIN encoding, RLE/bit-packed
definition levels for optional columns, per-column statistics and a
Thrift-compact footer. Physical types are BOOLEAN, INT64, DOUBLE and
BYTE_ARRAY/UTF8; integers are always INT64 (widening is lossless, narrowing is
not). **Dates are ISO-8601 strings**, not DATE-tagged int32s — every consumer
agrees on the string, while an epoch re-basing is how datasets drift. Conflicts
resolve upward: int + float ⇒ DOUBLE, anything + string ⇒ BYTE_ARRAY.

## XML

```xml
<harvest-export version="1" dataset="demo-run" records="2" generated_ts="…">
  <metadata>…</metadata>
  <records>
    <record index="0">
      <field name="url" type="text">https://…</field>
      <field name="qty" type="int">3</field>
      <field name="note" type="null" null="true"/>
    </record>
  </records>
</harvest-export>
```

Element names come from your data, so they are validated against the XML `Name`
production: illegal characters are repaired deterministically (`a b` → `a_b`,
`1bad` → `_1bad`) with the original kept in `source="…"`, and names that cannot
be repaired safely (the reserved `xml` prefix, for example) are **rejected** and
listed as `skipped-column`. A silently renamed column is a data-integrity bug
waiting to happen.

## ZIP bundle

Members are the files the manifest already describes, plus `manifest.json`, its
signature when `--key` was given, and an in-archive `bundle-manifest.json` that
names every member with its digest. Member names go through a path-safety check
that refuses absolute paths, drive letters, `..` traversal, backslashes, control
characters and over-long segments.

The bundle is deliberately **not** listed in the folder manifest — a folder
manifest cannot describe the archive that contains it. Its digest is printed by
the CLI and recorded in `bundle-manifest.json`.

## Verify

```
harvest verify --dir ./delivery          # presence, bytes, sha256, line counts, record total
harvest verify --dir ./delivery --key k  # + detached manifest signature
```

Record totals are re-counted from the JSONL payload when the delivery includes
one; for format sets without JSONL the declared total is carried and the verdict
reports `records_basis: manifest-declared`, because a workbook cannot be
re-counted without parsing it. Byte integrity is checked either way. Tampering
with any payload, the manifest or `checksums.txt` fails the verdict.

## Test coverage

`tests/export_formats.rs` writes all seven formats through `export::run` and
asserts: every file exists and is non-empty, is listed in the manifest with a
matching digest, is well-formed for its type (ZIP package structure and required
OOXML parts, `%PDF` header and trailer, `PAR1` magic and footer length, XML
element balance), the bundle has no traversal member names, byte-identical
re-runs for the deterministic formats, and a failed verdict after single-byte
tampering with any file.
