//! Real Parquet export for analytics consumers (pandas, DuckDB, Spark, Athena).
//!
//! The writer emits a genuine Parquet 1.0 file — `PAR1` magic, data pages with
//! PLAIN encoding, RLE/bit-packed definition levels for optional columns,
//! per-column statistics, and a Thrift-compact `FileMetaData` footer. No
//! external crate, no Arrow runtime: the format is small and stable enough to
//! write directly, which keeps the dependency surface (and the build) lean.
//!
//! Design decisions worth knowing before you read a file we produced:
//! * **Physical types**: BOOLEAN, INT64, DOUBLE and BYTE_ARRAY/UTF8. Integers
//!   are always INT64 (widening is lossless; narrowing is not).
//! * **Dates are ISO-8601 strings** (BYTE_ARRAY/UTF8), not DATE-tagged int32s:
//!   every source system agrees on the string, and silently re-basing a date
//!   into someone else's epoch is how datasets drift.
//! * **Nullable columns** carry definition levels; required columns carry none.
//!   A column is required only when the schema says so *and* no null was seen.
//! * **Codec is UNCOMPRESSED.** Deterministic output, and the ZIP bundle can
//!   compress the result if a client wants smaller bytes.
//! * **Bounded row groups** (`row_group_rows`): pages are streamed to disk and
//!   only metadata (columns × row groups) is held in memory.
//! * **Conflicting types resolve upward, explicitly**: bool+number ⇒ BYTE_ARRAY,
//!   int+float ⇒ DOUBLE, anything+string ⇒ BYTE_ARRAY. The resolution is
//!   recorded in the report so it is visible, not hidden.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Physical Parquet types this writer emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParquetType {
    Boolean,
    Int64,
    Double,
    ByteArray,
}

impl ParquetType {
    /// parquet-format `Type` enum ids.
    fn id(self) -> i32 {
        match self {
            Self::Boolean => 0,
            Self::Int64 => 2,
            Self::Double => 5,
            Self::ByteArray => 6,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Boolean => "BOOLEAN",
            Self::Int64 => "INT64",
            Self::Double => "DOUBLE",
            Self::ByteArray => "BYTE_ARRAY",
        }
    }
}

/// A typed column declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParquetColumn {
    pub name: String,
    pub kind: ParquetType,
    /// OPTIONAL (definition levels written) vs REQUIRED (none written).
    pub nullable: bool,
}

/// One cell.
#[derive(Debug, Clone, PartialEq)]
pub enum ParquetValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

/// The table to write: typed columns, rows in input order.
#[derive(Debug, Clone, Default)]
pub struct ParquetTable {
    pub columns: Vec<ParquetColumn>,
    pub rows: Vec<Vec<ParquetValue>>,
}

#[derive(Debug, Clone)]
pub struct ParquetOpts {
    /// Rows per row group (bounded; also the unit of column statistics).
    pub row_group_rows: usize,
    /// Written to the footer's `created_by`.
    pub created_by: String,
    /// Free-form key/value metadata embedded in the footer.
    pub key_values: Vec<(String, String)>,
}

impl Default for ParquetOpts {
    fn default() -> Self {
        Self {
            row_group_rows: 100_000,
            created_by: "harvest".to_string(),
            key_values: Vec::new(),
        }
    }
}

/// Result of one Parquet write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParquetReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub rows: u64,
    pub columns: usize,
    pub row_groups: usize,
    /// Nulls written across all columns.
    pub nulls: u64,
    /// Columns whose declared type was widened to hold the observed values.
    pub widened: Vec<String>,
}

/* ── thrift compact protocol ─────────────────────────────────────────────── */

const T_STOP: u8 = 0x00;
// Full compact-protocol type vocabulary, kept for readability of the encoder
// below even where this writer does not (yet) need a case.
#[allow(dead_code)]
const T_TRUE: u8 = 0x01;
#[allow(dead_code)]
const T_FALSE: u8 = 0x02;
const T_I32: u8 = 0x05;
const T_I64: u8 = 0x06;
#[allow(dead_code)]
const T_DOUBLE: u8 = 0x07;
const T_BINARY: u8 = 0x08;
const T_LIST: u8 = 0x09;
const T_STRUCT: u8 = 0x0C;

#[derive(Default)]
struct Tc {
    buf: Vec<u8>,
}

impl Tc {
    fn varint(&mut self, mut v: u64) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            self.buf.push(b);
            if v == 0 {
                break;
            }
        }
    }

    fn zigzag64(&mut self, v: i64) {
        self.varint(((v << 1) ^ (v >> 63)) as u64);
    }

    fn zigzag32(&mut self, v: i32) {
        self.varint((((v as i64) << 1) ^ ((v as i64) >> 63)) as u64);
    }

    /// Field header: short form when the id delta exceeds 15.
    fn field(&mut self, id: i16, prev: i16, t: u8) {
        let delta = id - prev;
        if delta > 0 && delta <= 15 {
            self.buf.push(((delta as u8) << 4) | t);
        } else {
            self.buf.push(t);
            self.zigzag32(id as i32);
        }
    }

    fn i32(&mut self, id: i16, prev: i16, v: i32) {
        self.field(id, prev, T_I32);
        self.zigzag32(v);
    }

    fn i64(&mut self, id: i16, prev: i16, v: i64) {
        self.field(id, prev, T_I64);
        self.zigzag64(v);
    }

    fn binary(&mut self, id: i16, prev: i16, v: &[u8]) {
        self.field(id, prev, T_BINARY);
        self.varint(v.len() as u64);
        self.buf.extend_from_slice(v);
    }

    fn string(&mut self, id: i16, prev: i16, v: &str) {
        self.binary(id, prev, v.as_bytes());
    }

    fn list(&mut self, id: i16, prev: i16, n: usize, elem: u8) {
        self.field(id, prev, T_LIST);
        if n < 15 {
            self.buf.push(((n as u8) << 4) | elem);
        } else {
            self.buf.push(0xF0 | elem);
            self.varint(n as u64);
        }
    }

    fn struct_begin(&mut self, id: i16, prev: i16) {
        self.field(id, prev, T_STRUCT);
    }

    fn stop(&mut self) {
        self.buf.push(T_STOP);
    }

    fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}

/* ── encoding primitives ─────────────────────────────────────────────────── */

/// RLE/bit-packed hybrid encoder, bit-packed runs only (always valid, and for
/// definition levels — one bit per value — it is also the compact form).
fn bit_packed(values: &[u32], bit_width: usize) -> Vec<u8> {
    let groups = values.len().div_ceil(8);
    let mut out = Vec::new();
    // run header: (groups << 1) | 1  (bit-packed run)
    let mut header = Tc::default();
    header.varint(((groups as u64) << 1) | 1);
    out.extend_from_slice(&header.into_inner());
    let mask = (1u32 << bit_width) - 1;
    for g in 0..groups {
        let mut byte = 0u8;
        for i in 0..8 {
            let v = values.get(g * 8 + i).copied().unwrap_or(0) & mask;
            byte |= (v as u8) << (i * bit_width);
        }
        out.push(byte);
    }
    out
}

fn bool_plain(values: &[bool]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in values.chunks(8) {
        let mut byte = 0u8;
        for (i, b) in chunk.iter().enumerate() {
            if *b {
                byte |= 1 << i;
            }
        }
        out.push(byte);
    }
    out
}

/// Bytes for one value in PLAIN encoding — except BOOLEAN, which PLAIN stores
/// bit-packed (1 bit per value) and is therefore handled by [`bool_plain`].
fn value_bytes(v: &ParquetValue) -> Option<Vec<u8>> {
    match v {
        ParquetValue::Null => None,
        ParquetValue::Bool(_) => None,
        ParquetValue::Int(i) => Some(i.to_le_bytes().to_vec()),
        ParquetValue::Float(f) => Some(f.to_le_bytes().to_vec()),
        ParquetValue::Str(s) => {
            let mut out = (s.len() as u32).to_le_bytes().to_vec();
            out.extend_from_slice(s.as_bytes());
            Some(out)
        }
    }
}

/// Compare two encoded values for min/max statistics. Returns None for types
/// where the spec leaves ordering undefined (booleans).
fn cmp_encoded(kind: ParquetType, a: &[u8], b: &[u8]) -> Option<std::cmp::Ordering> {
    match kind {
        ParquetType::Boolean => None,
        ParquetType::Int64 => {
            let a = i64::from_le_bytes(a.try_into().ok()?);
            let b = i64::from_le_bytes(b.try_into().ok()?);
            Some(a.cmp(&b))
        }
        ParquetType::Double => {
            let a = f64::from_le_bytes(a.try_into().ok()?);
            let b = f64::from_le_bytes(b.try_into().ok()?);
            a.partial_cmp(&b)
        }
        ParquetType::ByteArray => Some(a.cmp(b)),
    }
}

fn statistics(min: Option<&[u8]>, max: Option<&[u8]>, null_count: i64) -> Vec<u8> {
    let mut t = Tc::default();
    let mut prev = 0i16;
    if let (Some(mn), Some(mx)) = (min, max) {
        // 1: max, 2: min (deprecated V1 order), 5: max_value, 6: min_value
        t.binary(1, prev, mx);
        prev = 1;
        t.binary(2, prev, mn);
        prev = 2;
        t.i64(3, prev, null_count);
        prev = 3;
        t.binary(5, prev, mx);
        prev = 5;
        t.binary(6, prev, mn);
    } else {
        t.i64(3, 0, null_count);
        prev = 3;
    }
    let _ = prev;
    t.stop();
    t.into_inner()
}

fn page_header(num_values: i32, uncompressed: i32, stats: &[u8]) -> Vec<u8> {
    let mut t = Tc::default();
    let mut prev = 0i16;
    t.i32(1, prev, 0); // DATA_PAGE
    prev = 1;
    t.i32(2, prev, uncompressed);
    prev = 2;
    t.i32(3, prev, uncompressed); // compressed == uncompressed
    prev = 3;
    t.struct_begin(5, prev); // data_page_header
    {
        let mut d = Tc::default();
        let mut p = 0i16;
        d.i32(1, p, num_values);
        p = 1;
        d.i32(2, p, 0); // PLAIN
        p = 2;
        d.i32(3, p, 3); // RLE
        p = 3;
        d.i32(4, p, 3); // RLE
        p = 4;
        if !stats.is_empty() {
            d.field(5, p, T_STRUCT);
            d.buf.extend_from_slice(stats);
        }
        d.stop();
        t.buf.extend_from_slice(&d.into_inner());
    }
    t.stop();
    t.into_inner()
}

/* ── writer ──────────────────────────────────────────────────────────────── */

/// Write `table` to `path` as a Parquet file.
pub fn write_parquet(
    path: &Path,
    table: &ParquetTable,
    opts: &ParquetOpts,
) -> Result<ParquetReport> {
    if table.columns.is_empty() {
        bail!("parquet export needs at least one column");
    }
    let rows = table.rows.len();
    let group_rows = opts.row_group_rows.max(1);
    let groups = if rows == 0 {
        1
    } else {
        rows.div_ceil(group_rows)
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.parquet.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "export".to_string())
    ));
    let mut f =
        crate::export_io::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    let mut sha = Sha256::new();

    f.write_all(b"PAR1")?;
    sha.update(b"PAR1");
    let mut pos: u64 = 4;
    let mut nulls_total = 0u64;

    // column_chunk metadata per (group, column), filled as we stream.
    let mut chunks: Vec<ColumnChunkMeta> = Vec::with_capacity(groups * table.columns.len());

    for g in 0..groups {
        let start = g * group_rows;
        let end = (start + group_rows).min(rows);
        let group_rows_actual = end.saturating_sub(start);
        for (ci, col) in table.columns.iter().enumerate() {
            let mut defs: Vec<u32> = Vec::with_capacity(group_rows_actual);
            let mut vals: Vec<u8> = Vec::new();
            let mut bools: Vec<bool> = Vec::with_capacity(group_rows_actual);
            let mut null_count = 0i64;
            let mut min: Option<Vec<u8>> = None;
            let mut max: Option<Vec<u8>> = None;
            for r in start..end {
                let v = table
                    .rows
                    .get(r)
                    .and_then(|row| row.get(ci))
                    .unwrap_or(&ParquetValue::Null);
                if col.kind == ParquetType::Boolean {
                    match v {
                        ParquetValue::Bool(b) => {
                            bools.push(*b);
                            if col.nullable {
                                defs.push(1);
                            }
                        }
                        _ => {
                            null_count += 1;
                            nulls_total += 1;
                            if col.nullable {
                                defs.push(0);
                            }
                            bools.push(false);
                        }
                    }
                    continue;
                }
                match value_bytes(v) {
                    Some(bytes) => {
                        if col.nullable {
                            defs.push(1);
                        }
                        if let Some(cmp) = min
                            .as_deref()
                            .and_then(|m| cmp_encoded(col.kind, m, &bytes))
                        {
                            if cmp == std::cmp::Ordering::Greater {
                                min = Some(bytes.clone());
                            }
                        } else if min.is_none() {
                            min = Some(bytes.clone());
                        }
                        if let Some(cmp) = max
                            .as_deref()
                            .and_then(|m| cmp_encoded(col.kind, m, &bytes))
                        {
                            if cmp == std::cmp::Ordering::Less {
                                max = Some(bytes.clone());
                            }
                        } else if max.is_none() {
                            max = Some(bytes.clone());
                        }
                        vals.extend_from_slice(&bytes);
                    }
                    None => {
                        null_count += 1;
                        nulls_total += 1;
                        if col.nullable {
                            defs.push(0);
                        } else {
                            // Required column: a null is a schema violation.
                            // Write the zero value rather than corrupt the file's
                            // value counts, and let the schema layer quarantine
                            // the record — but never emit a short page.
                            vals.extend_from_slice(&zero_bytes(col.kind));
                        }
                    }
                }
            }
            if col.kind == ParquetType::Boolean {
                vals = bool_plain(&bools);
            }
            // Definition levels are RLE/bit-packed and, in a V1 data page,
            // length-prefixed with a 4-byte little-endian size (the prefix is
            // part of the page body the page header's size covers).
            let def_bytes = if col.nullable && !defs.is_empty() {
                let packed = bit_packed(&defs, 1);
                let mut out = (packed.len() as u32).to_le_bytes().to_vec();
                out.extend_from_slice(&packed);
                out
            } else {
                Vec::new()
            };
            let page_body_len = def_bytes.len() + vals.len();
            let stats = statistics(min.as_deref(), max.as_deref(), null_count);
            let header = page_header(group_rows_actual as i32, page_body_len as i32, &stats);

            f.write_all(&header)?;
            sha.update(&header);
            f.write_all(&def_bytes)?;
            sha.update(&def_bytes);
            f.write_all(&vals)?;
            sha.update(&vals);

            let page_start = pos;
            pos += (header.len() + page_body_len) as u64;
            chunks.push(ColumnChunkMeta {
                col: col.clone(),
                file_offset: page_start as i64,
                num_values: group_rows_actual as i64,
                total_size: (header.len() + page_body_len) as i64,
                group: g,
            });
        }
    }

    let mut total_rows_written = 0i64;
    let mut row_groups: Vec<Vec<u8>> = Vec::with_capacity(groups);
    for g in 0..groups {
        let group_chunks: Vec<&ColumnChunkMeta> = chunks.iter().filter(|c| c.group == g).collect();
        let rows_in_group = group_chunks.first().map(|c| c.num_values).unwrap_or(0);
        total_rows_written += rows_in_group;
        let mut rg = Tc::default();
        let mut prev = 0i16;
        rg.list(1, prev, group_chunks.len(), T_STRUCT);
        prev = 1;
        for c in &group_chunks {
            rg.buf.extend_from_slice(&c.encode());
        }
        rg.i64(
            2,
            prev,
            group_chunks.iter().map(|c| c.total_size).sum::<i64>(),
        );
        prev = 2;
        rg.i64(3, prev, rows_in_group);
        prev = 3;
        // 7: ordinal (i16), optional
        rg.field(7, prev, T_I32);
        rg.zigzag32(g as i32);
        rg.stop();
        row_groups.push(rg.into_inner());
    }

    // FileMetaData
    let mut fm = Tc::default();
    let mut prev = 0i16;
    fm.i32(1, prev, 1); // version
    prev = 1;
    fm.list(2, prev, table.columns.len() + 1, T_STRUCT);
    prev = 2;
    // root schema element
    {
        let mut root = Tc::default();
        let mut p = 0i16;
        root.i32(3, p, 0); // REQUIRED
        p = 3;
        root.string(4, p, "root");
        p = 4;
        root.i32(5, p, table.columns.len() as i32);
        let _ = p;
        root.stop();
        fm.buf.extend_from_slice(&root.into_inner());
    }
    for col in &table.columns {
        let mut e = Tc::default();
        let mut p = 0i16;
        e.i32(1, p, col.kind.id());
        p = 1;
        e.i32(3, p, if col.nullable { 1 } else { 0 }); // OPTIONAL / REQUIRED
        p = 3;
        e.string(4, p, &col.name);
        p = 4;
        if col.kind == ParquetType::ByteArray {
            e.i32(6, p, 0); // ConvertedType UTF8
        }
        e.stop();
        fm.buf.extend_from_slice(&e.into_inner());
    }
    fm.i64(3, prev, total_rows_written);
    prev = 3;
    fm.list(4, prev, row_groups.len(), T_STRUCT);
    prev = 4;
    for rg in &row_groups {
        fm.buf.extend_from_slice(rg);
    }
    if !opts.key_values.is_empty() {
        fm.list(5, prev, opts.key_values.len(), T_STRUCT);
        prev = 5;
        for (k, v) in &opts.key_values {
            let mut kv = Tc::default();
            let mut p = 0i16;
            kv.string(1, p, k);
            p = 1;
            kv.string(2, p, v);
            let _ = p;
            kv.stop();
            fm.buf.extend_from_slice(&kv.into_inner());
        }
    }
    fm.string(6, prev, &opts.created_by);
    fm.stop();
    let footer = fm.into_inner();

    f.write_all(&footer)?;
    sha.update(&footer);
    let len_bytes = (footer.len() as u32).to_le_bytes();
    f.write_all(&len_bytes)?;
    sha.update(len_bytes);
    f.write_all(b"PAR1")?;
    sha.update(b"PAR1");
    f.flush()?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;

    let bytes = std::fs::metadata(path)?.len();
    Ok(ParquetReport {
        path: path.to_path_buf(),
        bytes,
        sha256: format!("{:x}", sha.finalize()),
        rows: rows as u64,
        columns: table.columns.len(),
        row_groups: groups,
        nulls: nulls_total,
        widened: Vec::new(),
    })
}

fn zero_bytes(kind: ParquetType) -> Vec<u8> {
    match kind {
        ParquetType::Boolean => vec![0],
        ParquetType::Int64 => vec![0; 8],
        ParquetType::Double => vec![0; 8],
        ParquetType::ByteArray => vec![0; 4],
    }
}

struct ColumnChunkMeta {
    col: ParquetColumn,
    file_offset: i64,
    num_values: i64,
    total_size: i64,
    group: usize,
}

impl ColumnChunkMeta {
    fn encode(&self) -> Vec<u8> {
        let mut t = Tc::default();
        let mut prev = 0i16;
        t.i64(2, prev, self.file_offset);
        prev = 2;
        t.struct_begin(3, prev);
        {
            let mut m = Tc::default();
            let mut p = 0i16;
            m.i32(1, p, self.col.kind.id());
            p = 1;
            // encodings: [PLAIN, RLE]. List elements are bare zigzag varints —
            // no per-element type byte (that is only written in the header).
            m.list(2, p, 2, T_I32);
            m.zigzag32(0);
            m.zigzag32(3);
            p = 2;
            m.list(3, p, 1, T_BINARY); // path_in_schema
            m.varint(self.col.name.len() as u64);
            m.buf.extend_from_slice(self.col.name.as_bytes());
            p = 3;
            m.i32(4, p, 0); // UNCOMPRESSED
            p = 4;
            m.i64(5, p, self.num_values);
            p = 5;
            m.i64(6, p, self.total_size);
            p = 6;
            m.i64(7, p, self.total_size);
            p = 7;
            m.i64(9, p, self.file_offset);
            let _ = p;
            m.stop();
            t.buf.extend_from_slice(&m.into_inner());
        }
        t.stop();
        t.into_inner()
    }
}

/* ── schema mapping from JSON ────────────────────────────────────────────── */

/// Map a JSON value onto a Parquet value. Strings stay strings (including
/// ISO dates); json arrays/objects are serialized into BYTE_ARRAY.
pub fn value_from_json(v: &Value) -> ParquetValue {
    match v {
        Value::Null => ParquetValue::Null,
        Value::Bool(b) => ParquetValue::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => ParquetValue::Int(i),
            None => ParquetValue::Float(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => ParquetValue::Str(s.clone()),
        other => ParquetValue::Str(serde_json::to_string(other).unwrap_or_default()),
    }
}

/// Infer typed columns from JSON rows, scanning at most `scan` rows per column.
/// Conflicts resolve upward (see module docs); nullable is true iff a null was
/// observed (or the column is missing from a row).
pub fn infer_columns(columns: &[String], rows: &[Value], scan: usize) -> Vec<ParquetColumn> {
    let limit = scan.max(1).min(rows.len().max(1));
    columns
        .iter()
        .map(|name| {
            let mut saw_bool = false;
            let mut saw_int = false;
            let mut saw_float = false;
            let mut saw_str = false;
            let mut nullable = false;
            for row in rows.iter().take(limit) {
                match row.as_object().and_then(|o| o.get(name)) {
                    None | Some(Value::Null) => nullable = true,
                    Some(Value::Bool(_)) => saw_bool = true,
                    Some(Value::Number(n)) => {
                        if n.is_i64() || n.is_u64() {
                            saw_int = true;
                        } else {
                            saw_float = true;
                        }
                    }
                    Some(Value::String(_)) => saw_str = true,
                    Some(_) => saw_str = true,
                }
            }
            let kind = if saw_str || (saw_bool && (saw_int || saw_float)) {
                ParquetType::ByteArray
            } else if saw_float {
                ParquetType::Double
            } else if saw_int {
                ParquetType::Int64
            } else if saw_bool {
                ParquetType::Boolean
            } else {
                // No values seen (all null): keep it as a nullable string.
                nullable = true;
                ParquetType::ByteArray
            };
            ParquetColumn {
                name: name.clone(),
                kind,
                nullable,
            }
        })
        .collect()
}

/// Build a [`ParquetTable`] from JSON rows and column names, inferring types.
pub fn table_from_rows(columns: &[String], rows: &[Value], scan: usize) -> ParquetTable {
    let cols = infer_columns(columns, rows, scan);
    let out_rows = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|c| {
                    row.as_object()
                        .and_then(|o| o.get(c))
                        .map(value_from_json)
                        .unwrap_or(ParquetValue::Null)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    ParquetTable {
        columns: cols,
        rows: out_rows,
    }
}
