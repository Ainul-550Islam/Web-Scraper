//! Real `.xlsx` export — a spreadsheet a client can open in Excel, LibreOffice
//! or a pandas notebook without a converter step.
//!
//! The writer emits the OOXML SpreadsheetML package directly (a ZIP of XML
//! parts, produced by [`crate::export::bundle`] — the crate's only ZIP
//! implementation) with typed cells:
//!
//! | JSON | XLSX cell |
//! |---|---|
//! | string | inline string (`t="inlineStr"`, XML-escaped) |
//! | integer | numeric, no style |
//! | float | numeric, no style |
//! | bool | `t="b"` + `0`/`1` |
//! | `null` | cell omitted (a blank cell, not the text "null") |
//! | ISO date string | numeric date serial + a `yyyy-mm-dd` style |
//! | array/object | inline string holding compact JSON |
//!
//! No shared-string table is used: inline strings keep the writer stateless
//! across sheets and make the output a pure function of the input rows (same
//! bytes for the same records, which is what the manifest promises).
//!
//! Bounds are enforced, not documented-and-hoped-for: rows per sheet, columns
//! per sheet, and characters per string all have caps, and exceeding the row
//! cap rolls over into additional sheets (`records`, `records-2`, …) that each
//! carry the header row. Finalization is atomic (temp file + rename), so an
//! interrupted export never leaves a half-written spreadsheet behind.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::export::bundle::{write_zip, Member};
use crate::export::xml::escape_xml;

/// Excel's own limits; our defaults sit well inside them.
pub const EXCEL_MAX_ROWS: usize = 1_048_576;
pub const EXCEL_MAX_COLS: usize = 16_384;
pub const EXCEL_MAX_CELL_CHARS: usize = 32_767;

/// Default caps. A delivery that needs more than a million rows per sheet is
/// better served by Parquet or chunked CSV.
#[derive(Debug, Clone, Copy)]
pub struct XlsxOpts {
    /// Data rows per worksheet (header excluded); extra rows roll to a new sheet.
    pub max_rows_per_sheet: usize,
    /// Columns written per row; the rest are dropped and counted.
    pub max_cols: usize,
    /// Characters kept per string cell.
    pub max_cell_chars: usize,
}

impl Default for XlsxOpts {
    fn default() -> Self {
        Self {
            max_rows_per_sheet: 100_000,
            max_cols: 256,
            max_cell_chars: 4_096,
        }
    }
}

/// A typed spreadsheet cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Text(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    /// Days since the 1899-12-30 epoch (Excel's date system).
    Date(i64),
    Null,
}

impl Cell {
    /// Map a JSON value onto a typed cell. Strings that parse as an ISO date
    /// become date cells — the one inference the writer makes, and it is
    /// reversible (the style is a display format over the same value).
    pub fn from_value(v: &Value) -> Self {
        match v {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Bool(*b),
            Value::Number(n) => match n.as_i64() {
                Some(i) => Self::Int(i),
                None => Self::Float(n.as_f64().unwrap_or(0.0)),
            },
            Value::String(s) => match parse_iso_date(s) {
                Some((y, m, d)) => Self::Date(date_serial(y, m, d)),
                None => Self::Text(s.clone()),
            },
            other => Self::Text(serde_json::to_string(other).unwrap_or_default()),
        }
    }
}

/// One logical table: a name, ordered columns, ordered rows.
#[derive(Debug, Clone, Default)]
pub struct Sheet {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

impl Sheet {
    /// Build a sheet from JSON rows: one column per key in `columns`, one row
    /// per record (missing keys become [`Cell::Null`]).
    pub fn from_rows(name: &str, columns: &[String], rows: &[Value]) -> Self {
        let mut out = Self {
            name: name.to_string(),
            columns: columns.to_vec(),
            rows: Vec::new(),
        };
        for row in rows {
            let obj = row.as_object();
            let mut cells = Vec::with_capacity(columns.len());
            for c in columns {
                let v = obj.and_then(|o| o.get(c)).unwrap_or(&Value::Null);
                cells.push(Cell::from_value(v));
            }
            out.rows.push(cells);
        }
        out
    }
}

/// What was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XlsxReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub sheets: usize,
    /// Data rows written (header rows excluded).
    pub rows: u64,
    pub columns: usize,
    /// Columns dropped because they exceeded `max_cols`.
    pub columns_dropped: usize,
    /// Strings shortened to `max_cell_chars`.
    pub truncated_strings: u64,
}

/// Write `sheet` to `path` as a valid `.xlsx`.
pub fn write_xlsx(path: &Path, sheet: &Sheet, opts: &XlsxOpts) -> Result<XlsxReport> {
    if sheet.columns.is_empty() {
        bail!("xlsx export needs at least one column");
    }
    let max_rows = opts.max_rows_per_sheet.clamp(1, EXCEL_MAX_ROWS - 1);
    let max_cols = opts.max_cols.clamp(1, EXCEL_MAX_COLS);
    let max_chars = opts.max_cell_chars.clamp(1, EXCEL_MAX_CELL_CHARS);

    let columns = &sheet.columns[..sheet.columns.len().min(max_cols)];
    let columns_dropped = sheet.columns.len().saturating_sub(columns.len());

    let chunks: Vec<&[Vec<Cell>]> = if sheet.rows.is_empty() {
        vec![&sheet.rows[..]]
    } else {
        sheet.rows.chunks(max_rows).collect()
    };
    if chunks.is_empty() {
        bail!("xlsx export produced no sheets");
    }

    let base = safe_sheet_name(&sheet.name);
    let mut sheet_names: Vec<String> = Vec::with_capacity(chunks.len());
    for i in 0..chunks.len() {
        let name = if i == 0 {
            base.clone()
        } else {
            safe_sheet_name(&format!("{}-{}", base, i + 1))
        };
        sheet_names.push(unique_name(name, &sheet_names));
    }

    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut truncated = 0u64;
    let mut rows_written = 0u64;
    for (i, rows) in chunks.iter().enumerate() {
        let mut xml = String::with_capacity(512 + rows.len() * 64);
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        xml.push_str("<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>");
        push_row(
            &mut xml,
            1,
            columns.iter().map(|c| Cell::Text(c.clone())).collect(),
            true,
            max_chars,
            columns.len(),
            &mut truncated,
        );
        for (r, row) in rows.iter().enumerate() {
            push_row(
                &mut xml,
                (r + 2) as u64,
                row.clone(),
                false,
                max_chars,
                columns.len(),
                &mut truncated,
            );
            let retained: usize = parts.iter().map(|(_, body)| body.len()).sum();
            if retained.saturating_add(xml.len()) as u64 > crate::export_io::remaining() {
                bail!("XLSX exceeds the export output byte budget");
            }
            crate::work_cancel::check()?;
        }
        xml.push_str("</sheetData></worksheet>");
        rows_written += rows.len() as u64;
        parts.push((
            format!("xl/worksheets/sheet{}.xml", i + 1),
            xml.into_bytes(),
        ));
    }

    parts.push((
        "[Content_Types].xml".to_string(),
        content_types(chunks.len()).into_bytes(),
    ));
    parts.push(("_rels/.rels".to_string(), root_rels().into_bytes()));
    parts.push((
        "xl/workbook.xml".to_string(),
        workbook(&sheet_names).into_bytes(),
    ));
    parts.push((
        "xl/_rels/workbook.xml.rels".to_string(),
        workbook_rels(chunks.len()).into_bytes(),
    ));
    parts.push(("xl/styles.xml".to_string(), styles().into_bytes()));

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let members: Vec<(String, Member<'_>)> = parts
        .iter()
        .map(|(name, bytes)| (name.clone(), Member::Bytes(bytes)))
        .collect();
    let report = write_zip(path, &members)?;

    Ok(XlsxReport {
        path: report.path,
        bytes: report.bytes,
        sha256: report.sha256,
        sheets: chunks.len(),
        rows: rows_written,
        columns: columns.len(),
        columns_dropped,
        truncated_strings: truncated,
    })
}

#[allow(clippy::too_many_arguments)]
fn push_row(
    xml: &mut String,
    row_no: u64,
    cells: Vec<Cell>,
    header: bool,
    max_chars: usize,
    max_cols: usize,
    truncated: &mut u64,
) {
    xml.push_str(&format!("<row r=\"{row_no}\">"));
    for (idx, cell) in cells.into_iter().take(max_cols).enumerate() {
        let col = col_letter(idx);
        let r = format!("{col}{row_no}");
        match cell {
            Cell::Null => { /* blank cell: nothing to write */ }
            Cell::Text(mut s) => {
                if s.encode_utf16().count() > max_chars {
                    let mut units = 0usize;
                    s = s
                        .chars()
                        .take_while(|c| {
                            units += c.len_utf16();
                            units <= max_chars
                        })
                        .collect();
                    *truncated += 1;
                }
                xml.push_str(&format!(
                    "<c r=\"{r}\"{} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                    cell_style(header, false),
                    escape_xml(&s)
                ));
            }
            Cell::Int(v) => xml.push_str(&format!(
                "<c r=\"{r}\"{}><v>{}</v></c>",
                cell_style(header, false),
                v
            )),
            Cell::Float(v) => xml.push_str(&format!(
                "<c r=\"{r}\"{}><v>{}</v></c>",
                cell_style(header, false),
                fmt_f64(v)
            )),
            Cell::Bool(b) => xml.push_str(&format!(
                "<c r=\"{r}\"{} t=\"b\"><v>{}</v></c>",
                cell_style(header, false),
                u8::from(b)
            )),
            Cell::Date(d) => xml.push_str(&format!(
                "<c r=\"{r}\"{}><v>{}</v></c>",
                cell_style(header, true),
                d
            )),
        }
    }
    xml.push_str("</row>");
}

fn cell_style(header: bool, is_date: bool) -> &'static str {
    if header {
        " s=\"1\""
    } else if is_date {
        " s=\"2\""
    } else {
        ""
    }
}

/// Deterministic float formatting: no `1e20`, no locale commas, no `-0`.
fn fmt_f64(v: f64) -> String {
    if v.is_nan() || v.is_infinite() {
        return "0".to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    format!("{v:?}")
}

/// Column index (0-based) to SpreadsheetML letters: 0 → A, 25 → Z, 26 → AA.
pub fn col_letter(idx: usize) -> String {
    let mut n = idx + 1;
    let mut out = String::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.insert(0, (b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    out
}

/// Excel workbook sheet names: ≤ 31 chars, no `[]:*?/\`, non-empty.
pub fn safe_sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return "Sheet1".to_string();
    }
    cleaned.chars().take(31).collect()
}

fn unique_name(name: String, taken: &[String]) -> String {
    if !taken.contains(&name) {
        return name;
    }
    let mut i = 2;
    loop {
        let candidate = format!("{}-{}", name.chars().take(27).collect::<String>(), i);
        if !taken.contains(&candidate) {
            return safe_sheet_name(&candidate);
        }
        i += 1;
    }
}

/// Days since Excel's 1899-12-30 epoch (the 1900 date system, with the
/// Lotus 1900-leap-year bug baked in, as every spreadsheet expects).
pub fn date_serial(y: i32, m: u32, d: u32) -> i64 {
    let yy = y as i64;
    let (yy, mm) = if m < 3 {
        (yy - 1, m as i64 + 12)
    } else {
        (yy, m as i64)
    };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400; // [0, 399]
    let doy = (153 * (mm - 3) + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146_097 + doe - 719_468; // days since 1970-01-01
    days + 25_569 // 1899-12-30 epoch
}

/// Parse a strict `YYYY-MM-DD` (optionally `THH:MM…`) prefix.
pub fn parse_iso_date(s: &str) -> Option<(i32, u32, u32)> {
    let t = s.trim();
    if t.len() < 10 {
        return None;
    }
    let b = t.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: i32 = t[0..4].parse().ok()?;
    let m: u32 = t[5..7].parse().ok()?;
    let d: u32 = t[8..10].parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(0..=9999).contains(&y) {
        return None;
    }
    Some((y, m, d))
}

fn content_types(sheets: usize) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
    );
    for i in 1..=sheets {
        s.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
    }
    s.push_str("<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");
    s.push_str("</Types>");
    s
}

fn root_rels() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
     <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
     <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
     </Relationships>"
        .to_string()
}

fn workbook(names: &[String]) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>",
    );
    for (i, n) in names.iter().enumerate() {
        s.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
            escape_xml(n),
            i + 1,
            i + 1
        ));
    }
    s.push_str("</sheets></workbook>");
    s
}

fn workbook_rels(sheets: usize) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for i in 1..=sheets {
        s.push_str(&format!(
            "<Relationship Id=\"rId{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{i}.xml\"/>"
        ));
    }
    s.push_str(&format!(
        "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
        sheets + 1
    ));
    s.push_str("</Relationships>");
    s
}

/// Minimal styles: default, bold header, and one date number format.
fn styles() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
     <styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
     <numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"yyyy\\-mm\\-dd\"/></numFmts>\
     <fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font>\
     <font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts>\
     <fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill>\
     <fill><patternFill patternType=\"gray125\"/></fill></fills>\
     <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\
     <cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
     <cellXfs count=\"3\">\
     <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
     <xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/>\
     <xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>\
     </cellXfs>\
     <cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
     </styleSheet>"
        .to_string()
}
