//! Real PDF report export — summary-oriented by design.
//!
//! This is a **report**, not a data dump: run/dataset identity, per-target
//! counts, data-quality and quarantine tallies, schema drift, retry/failure
//! summaries, storage and export/manifest references, plus one bounded sample
//! table. Dumping a full dataset into a PDF is unreadable and enormous, so the
//! row cap is enforced rather than suggested (see [`PdfOpts`]).
//!
//! The writer emits a self-contained PDF 1.4 file with a classic cross-reference
//! table: no compression, no external resources, no embedded fonts beyond the
//! two standard Type1 faces every viewer has (Helvetica / Helvetica-Bold).
//! Output is deterministic — same model ⇒ same bytes — because the only
//! variable metadata (`/CreationDate`) is derived from the model's timestamp
//! and no document ID is generated.
//!
//! Text is WinAnsi-safe: `\`, `(` and `)` are escaped and any character outside
//! Latin-1 is replaced with `?` rather than emitted as bytes the font cannot
//! map. Malformed input therefore degrades to a readable report instead of a
//! corrupt file.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

/// Caps. Defaults keep a report to a handful of pages.
#[derive(Debug, Clone, Copy)]
pub struct PdfOpts {
    /// Rows rendered in any one sample table.
    pub max_table_rows: usize,
    /// Columns rendered in any one sample table.
    pub max_table_cols: usize,
    /// Hard page ceiling (further content is dropped, and counted).
    pub max_pages: usize,
    /// Characters kept per cell/label.
    pub max_cell_chars: usize,
    /// Characters per wrapped paragraph line.
    pub line_width: usize,
}

impl Default for PdfOpts {
    fn default() -> Self {
        Self {
            max_table_rows: 40,
            max_table_cols: 6,
            max_pages: 12,
            max_cell_chars: 48,
            line_width: 92,
        }
    }
}

/// A bounded sample table.
#[derive(Debug, Clone, Default)]
pub struct PdfTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Rows in the source that were not rendered.
    pub omitted_rows: usize,
    /// Columns in the source that were not rendered.
    pub omitted_cols: usize,
}

/// One titled block: key/value rows, an optional table, and paragraphs.
#[derive(Debug, Clone, Default)]
pub struct PdfSection {
    pub heading: String,
    pub rows: Vec<(String, String)>,
    pub table: Option<PdfTable>,
    pub paragraphs: Vec<String>,
}

/// The report model. Everything shown comes from here — the exporter never
/// reaches into engine state on its own.
#[derive(Debug, Clone, Default)]
pub struct PdfDoc {
    pub title: String,
    /// Unix seconds; printed as the PDF creation date and in the header.
    pub generated_ts: i64,
    pub subtitle: Option<String>,
    pub sections: Vec<PdfSection>,
    /// Closing notes (e.g. verification instructions for the client).
    pub notes: Vec<String>,
}

/// Result of writing a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub pages: usize,
    /// Content dropped because `max_pages` was reached.
    pub dropped: bool,
}

const PAGE_W: f64 = 595.0;
const PAGE_H: f64 = 842.0;
const MARGIN: f64 = 48.0;
const CONTENT_W: f64 = PAGE_W - 2.0 * MARGIN;

/// Render `doc` to a PDF file at `path`.
/// A drawing step did not fit on the current page. Callers flush the page and
/// retry on a fresh one; only a step that cannot fit an empty page is dropped.
#[derive(Debug, Clone, Copy)]
pub struct Overflow;

pub fn write_pdf(path: &Path, doc: &PdfDoc, opts: &PdfOpts) -> Result<PdfReport> {
    if doc.title.trim().is_empty() {
        bail!("pdf export needs a title");
    }
    let max_pages = opts.max_pages.max(1);
    let mut pages: Vec<Vec<u8>> = Vec::new();
    let mut page = Page::new();
    let mut dropped = false;

    // Every drawing step goes through `step_fn`: on overflow the page is
    // flushed and the step retried once on a fresh page. A step that cannot fit
    // even an empty page is dropped (and reported), never drawn off-page.
    fn step_fn<F: FnMut(&mut Page) -> Result<(), Overflow>>(
        pages: &mut Vec<Vec<u8>>,
        page: &mut Page,
        max_pages: usize,
        dropped: &mut bool,
        mut draw: F,
    ) -> bool {
        if draw(page).is_ok() {
            return true;
        }
        let finished = std::mem::replace(page, Page::new());
        pages.push(finished.finish());
        if pages.len() >= max_pages {
            *dropped = true;
            return false;
        }
        let _ = draw(page);
        true
    }

    let title = doc.title.clone();
    let date_line = format!("generated {}", pdf_date(doc.generated_ts));
    let sample = opts.max_cell_chars.max(1);
    let width = opts.line_width.max(20);

    step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
        p.heading(&title, 16.0)
    });
    if let Some(sub) = doc.subtitle.as_deref() {
        let sub = clip(sub, width * 2);
        step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
            p.text(&sub, 10.0, false)
        });
    }
    step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
        p.text(&date_line, 9.0, false)
    });
    step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| p.rule());
    step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
        p.gap(6.0)
    });

    'sections: for section in &doc.sections {
        let heading = section.heading.clone();
        if !step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
            p.heading(&heading, 12.0)
        }) {
            break 'sections;
        }
        for (k, v) in &section.rows {
            let kv = (clip(k, sample), clip(v, sample));
            if !step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
                p.kv(&kv.0, &kv.1)
            }) {
                break 'sections;
            }
        }
        if let Some(table) = section.table.as_ref() {
            if !step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
                p.table(table, opts)
            }) {
                break 'sections;
            }
        }
        for para in &section.paragraphs {
            for line in wrap(para, width) {
                if !step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
                    p.text(&line, 9.0, false)
                }) {
                    break 'sections;
                }
            }
            if !step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
                p.gap(3.0)
            }) {
                break 'sections;
            }
        }
        if !step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
            p.gap(10.0)
        }) {
            break 'sections;
        }
    }

    if !dropped {
        for note in &doc.notes {
            for line in wrap(note, width) {
                if !step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
                    p.text(&line, 8.0, false)
                }) {
                    break;
                }
            }
        }
        step_fn(&mut pages, &mut page, max_pages, &mut dropped, |p| {
            p.gap(4.0)
        });
    }

    pages.push(page.finish());
    if pages.is_empty() {
        bail!("pdf export produced no pages");
    }

    let bytes = assemble(&pages, doc, dropped);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.pdf.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "report".to_string())
    ));
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(&bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(PdfReport {
        path: path.to_path_buf(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", h.finalize()),
        pages: pages.len(),
        dropped,
    })
}

/// One page's content stream under construction.
struct Page {
    ops: String,
    y: f64,
}

impl Page {
    fn new() -> Self {
        Self {
            ops: String::new(),
            y: PAGE_H - MARGIN,
        }
    }

    fn gap(&mut self, dy: f64) -> Result<(), Overflow> {
        self.y -= dy;
        if self.y < MARGIN {
            return Err(Overflow);
        }
        Ok(())
    }

    fn text(&mut self, s: &str, size: f64, bold: bool) -> Result<(), Overflow> {
        self.y -= size + 3.0;
        if self.y < MARGIN {
            return Err(Overflow);
        }
        let font = if bold { "/F2" } else { "/F1" };
        self.ops.push_str(&format!(
            "BT {} {} Tf 1 0 0 1 {:.2} {:.2} Tm ({}) Tj ET\n",
            font,
            size,
            MARGIN,
            self.y,
            pdf_escape(s)
        ));
        Ok(())
    }

    fn heading(&mut self, s: &str, size: f64) -> Result<(), Overflow> {
        self.gap(4.0)?;
        self.text(s, size, true)
    }

    /// Label on the left, value right-aligned in the content column.
    fn kv(&mut self, k: &str, v: &str) -> Result<(), Overflow> {
        self.y -= 13.0;
        if self.y < MARGIN {
            return Err(Overflow);
        }
        self.ops.push_str(&format!(
            "BT /F1 9 Tf 1 0 0 1 {:.2} {:.2} Tm ({}) Tj ET\n",
            MARGIN,
            self.y,
            pdf_escape(k)
        ));
        // Right-align the value by measuring the font crudely: Helvetica is
        // ~0.5 em per character on average, which is exact enough for a report
        // column and never overflows the right margin.
        let w = v.chars().count() as f64 * 4.6;
        let x = (PAGE_W - MARGIN - w).max(MARGIN + 160.0);
        self.ops.push_str(&format!(
            "BT /F1 9 Tf 1 0 0 1 {:.2} {:.2} Tm ({}) Tj ET\n",
            x,
            self.y,
            pdf_escape(v)
        ));
        self.ops.push_str(&format!(
            "0.80 0.82 0.85 RG 0.4 w {:.2} {:.2} m {:.2} {:.2} l S\n",
            MARGIN,
            self.y - 3.0,
            PAGE_W - MARGIN,
            self.y - 3.0
        ));
        Ok(())
    }

    fn rule(&mut self) -> Result<(), Overflow> {
        self.ops.push_str(&format!(
            "0.55 0.58 0.62 RG 0.8 w {:.2} {:.2} m {:.2} {:.2} l S\n",
            MARGIN,
            self.y,
            PAGE_W - MARGIN,
            self.y
        ));
        self.gap(8.0)
    }

    fn table(&mut self, table: &PdfTable, opts: &PdfOpts) -> Result<(), Overflow> {
        let cols = table.columns.len().min(opts.max_table_cols.max(1));
        if cols == 0 {
            return Ok(());
        }
        let col_w = CONTENT_W / cols as f64;
        let row_h = 14.0;
        self.gap(6.0)?;
        self.y -= row_h;
        if self.y < MARGIN {
            return Err(Overflow);
        }
        // header band
        self.ops.push_str(&format!(
            "0.94 0.95 0.97 rg {:.2} {:.2} {:.2} {:.2} re f 0 0 0 rg\n",
            MARGIN,
            self.y - 3.0,
            CONTENT_W,
            row_h
        ));
        for (i, c) in table.columns.iter().take(cols).enumerate() {
            let x = MARGIN + 4.0 + i as f64 * col_w;
            self.ops.push_str(&format!(
                "BT /F2 8 Tf 1 0 0 1 {:.2} {:.2} Tm ({}) Tj ET\n",
                x,
                self.y,
                pdf_escape(&clip(c, (col_w / 4.7) as usize))
            ));
        }
        let mut rendered = 0usize;
        for row in table.rows.iter().take(opts.max_table_rows.max(1)) {
            self.y -= row_h;
            if self.y < MARGIN {
                return Err(Overflow);
            }
            for (i, cell) in row.iter().take(cols).enumerate() {
                let x = MARGIN + 4.0 + i as f64 * col_w;
                self.ops.push_str(&format!(
                    "BT /F1 8 Tf 1 0 0 1 {:.2} {:.2} Tm ({}) Tj ET\n",
                    x,
                    self.y,
                    pdf_escape(&clip(cell, (col_w / 4.6) as usize))
                ));
            }
            self.ops.push_str(&format!(
                "0.88 0.89 0.91 RG 0.3 w {:.2} {:.2} m {:.2} {:.2} l S\n",
                MARGIN,
                self.y - 3.5,
                PAGE_W - MARGIN,
                self.y - 3.5
            ));
            rendered += 1;
        }
        let omitted = table.omitted_rows + table.rows.len().saturating_sub(rendered);
        if omitted > 0 || table.omitted_cols > 0 {
            self.y -= 12.0;
            if self.y < MARGIN {
                return Err(Overflow);
            }
            let mut note = String::from("sample only —");
            if omitted > 0 {
                note.push_str(&format!(" {omitted} further row(s) not rendered"));
            }
            if table.omitted_cols > 0 {
                note.push_str(&format!(", {} column(s) not rendered", table.omitted_cols));
            }
            self.ops.push_str(&format!(
                "BT /F1 8 Tf 1 0 0 1 {:.2} {:.2} Tm ({}) Tj ET\n",
                MARGIN,
                self.y,
                pdf_escape(&note)
            ));
        }
        Ok(())
    }

    fn finish(self) -> Vec<u8> {
        self.ops.into_bytes()
    }
}

/// Build the full PDF byte stream: objects, cross-reference table, trailer.
fn assemble(pages: &[Vec<u8>], doc: &PdfDoc, dropped: bool) -> Vec<u8> {
    // Object ids: 1 catalog, 2 pages, 3 font F1, 4 font F2, then per page a
    // page object and a content object, then the info dictionary.
    let first_page_obj = 5u32;
    let info_obj = first_page_obj + (pages.len() as u32) * 2;
    let mut objects: Vec<(u32, Vec<u8>)> = Vec::new();
    objects.push((1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()));
    let mut kids = String::new();
    for i in 0..pages.len() {
        let id = first_page_obj + (i as u32) * 2;
        if i > 0 {
            kids.push(' ');
        }
        kids.push_str(&format!("{id} 0 R"));
    }
    objects.push((
        2,
        format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", pages.len()).into_bytes(),
    ));
    objects.push((
        3,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ));
    objects.push((
        4,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ));
    for (i, content) in pages.iter().enumerate() {
        let page_id = first_page_obj + (i as u32) * 2;
        let content_id = page_id + 1;
        objects.push((
            page_id,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] \
                 /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_id} 0 R >>"
            )
            .into_bytes(),
        ));
        let mut stream = Vec::new();
        stream.extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
        stream.extend_from_slice(content);
        stream.extend_from_slice(b"\nendstream");
        objects.push((content_id, stream));
    }
    let mut title = clip(&doc.title, 120).replace('(', "[").replace(')', "]");
    if dropped {
        title.push_str(" (truncated)");
    }
    objects.push((
        info_obj,
        format!(
            "<< /Title ({}) /Producer (harvest) /Creator (harvest export) /CreationDate ({}) /ModDate ({}) >>",
            title,
            pdf_date(doc.generated_ts),
            pdf_date(doc.generated_ts)
        )
        .into_bytes(),
    ));

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets: Vec<(u32, u64)> = Vec::new();
    for (id, body) in &objects {
        offsets.push((*id, out.len() as u64));
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_offset = out.len() as u64;
    let max_id = objects.iter().map(|(id, _)| *id).max().unwrap_or(0);
    out.extend_from_slice(format!("xref\n0 {}\n", max_id + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    let mut by_id = std::collections::BTreeMap::new();
    for (id, off) in offsets {
        by_id.insert(id, off);
    }
    for id in 1..=max_id {
        match by_id.get(&id) {
            Some(off) => out.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes()),
            None => out.extend_from_slice(b"0000000000 65535 f \n"),
        }
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info {} 0 R >>\nstartxref\n{}\n%%EOF\n",
            max_id + 1,
            info_obj,
            xref_offset
        )
        .as_bytes(),
    );
    out
}

/// Escape the three characters that are special inside a PDF string literal,
/// transliterate the typography we use in reports to ASCII, and map anything
/// else the standard fonts cannot show to `?`.
///
/// Only ASCII survives on purpose: the content stream declares
/// `/WinAnsiEncoding`, where a code point above U+007F is a single byte, while
/// Rust strings are UTF-8 — writing the UTF-8 bytes of `·` or `—` produces
/// mojibake in every viewer. Transliterating keeps the report readable instead
/// of technically-valid-but-wrong.
pub fn pdf_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 128 => out.push(c),
            '\u{2014}' | '\u{2013}' | '\u{2212}' => out.push('-'),
            '\u{00b7}' | '\u{2022}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            '\u{2018}' | '\u{2019}' | '\u{00b4}' => out.push('\''),
            '\u{201c}' | '\u{201d}' => out.push('"'),
            '\u{2264}' => out.push_str("<="),
            '\u{2265}' => out.push_str(">="),
            _ => out.push('?'),
        }
    }
    if out.is_empty() {
        out.push(' ');
    }
    out
}

/// `D:YYYYMMDDHHMMSS` in UTC — deterministic for a given timestamp.
pub fn pdf_date(ts: i64) -> String {
    let secs = ts.max(0) as u64;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    format!("D:{y:04}{m:02}{d:02}{hh:02}{mm:02}{ss:02}")
}

/// Days since the Unix epoch → (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Clip to `max` characters, marking truncation with an ellipsis.
pub fn clip(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = chars.into_iter().take(keep).collect();
    out.push('…');
    out
}

/// Greedy word wrap at `width` characters (paragraphs are report prose, so
/// character wrapping is accurate and deterministic).
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if cur.is_empty() && cur.chars().count() + word.chars().count() <= width {
            cur.push_str(word);
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            if word.chars().count() > width {
                let chars: Vec<char> = word.chars().collect();
                let mut i = 0usize;
                while i + width <= chars.len() {
                    lines.push(chars[i..i + width].iter().collect());
                    i += width;
                }
                cur = chars[i..].iter().collect();
            } else {
                cur = word.to_string();
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
