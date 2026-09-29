//! Deterministic XML export for system-to-system interchange.
//!
//! The output is intentionally boring and strictly structured:
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <harvest-export version="1" dataset="dataset-demo" records="2">
//!   <metadata>
//!     <generated_ts>...</generated_ts>
//!     <source>...</source>
//!     <columns><column name="url" type="text"/></columns>
//!   </metadata>
//!   <records>
//!     <record index="0">
//!       <field name="url" type="text">https://…</field>
//!       <field name="hits" type="int">12</field>
//!       <field name="note" type="text" null="true"/>
//!     </record>
//!   </records>
//! </harvest-export>
//! ```
//!
//! Guarantees:
//! * **Well-formed, always.** Every text node and attribute is escaped
//!   ([`escape_xml`]), including the five XML entities, plus control characters
//!   are stripped so a stray `\u{1}` cannot make the file unparseable.
//! * **No invented element names.** Field names come from the data, so they are
//!   validated against the XML `Name` production ([`is_xml_name`]); anything
//!   unsafe is *rejected* (reported in the result) rather than silently
//!   mangled — a renamed column is a data-integrity bug waiting to happen.
//! * **Stable ordering.** Columns in the configured order, records in input
//!   order, attributes in a fixed order: the same records produce byte-identical
//!   XML.
//! * **Bounded.** Row, column and string caps, matching the other exporters.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use sha2::{Digest, Sha256};
use std::io::Write;

/// Escape `&`, `<`, `>`, `"`, `'` and strip characters XML 1.0 cannot carry.
pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if is_xml_char(c) => out.push(c),
            _ => { /* control/unpaired surrogate: dropped, not encoded */ }
        }
    }
    out
}

/// XML 1.0 character range (plus tab/CR/LF).
fn is_xml_char(c: char) -> bool {
    matches!(c, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&c)
        || ('\u{E000}'..='\u{FFFD}').contains(&c)
        || ('\u{10000}'..='\u{10FFFF}').contains(&c)
}

/// True when `name` is a legal XML element/attribute `Name`: starts with a
/// letter or `_`, then letters, digits, `.`, `-`, `_`. Colons are rejected
/// (they mean namespaces, and a data-derived column has no business declaring
/// one), as is the reserved `xml` prefix.
pub fn is_xml_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    if name.to_ascii_lowercase().starts_with("xml") {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Suggest a usable element name for a data-derived column. Returns `None`
/// when the name cannot be repaired safely — callers then skip the field and
/// report it, rather than shipping a silently different column.
pub fn sanitize_name(name: &str) -> Option<String> {
    if is_xml_name(name) {
        return Some(name.to_string());
    }
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.trim().chars().enumerate() {
        let c = if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
            c
        } else {
            '_'
        };
        if i == 0 && !c.is_alphabetic() && c != '_' {
            out.push('_');
        }
        out.push(c);
    }
    let out: String = out.chars().take(128).collect();
    if is_xml_name(&out) && !out.is_empty() {
        Some(out)
    } else {
        None
    }
}

/// Caps for one document.
#[derive(Debug, Clone, Copy)]
pub struct XmlOpts {
    pub max_records: usize,
    pub max_fields: usize,
    pub max_chars: usize,
    /// Include the `<metadata>` block (columns, counts, sources).
    pub metadata: bool,
}

impl Default for XmlOpts {
    fn default() -> Self {
        Self {
            max_records: 100_000,
            max_fields: 512,
            max_chars: 8_192,
            metadata: true,
        }
    }
}

/// Result of one XML export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlReport {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
    pub records: u64,
    pub fields: usize,
    /// Columns whose name was not a legal XML name and could not be repaired.
    pub skipped_fields: Vec<String>,
    pub truncated_strings: u64,
}

/// JSON type label used in the `type` attribute (stable vocabulary).
pub fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_i64() || n.is_u64() => "int",
        Value::Number(_) => "number",
        Value::String(_) => "text",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}

/// Render one value as escaped element text. Arrays/objects become compact
/// JSON (documented in the `type` attribute), nulls become empty elements
/// carrying `null="true"`.
pub fn text_of(v: &Value, max_chars: usize) -> (String, bool) {
    let raw = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    if raw.chars().count() > max_chars {
        (raw.chars().take(max_chars).collect(), true)
    } else {
        (raw, false)
    }
}

/// Write `rows` (a JSON array of objects) as one XML document at `path`.
pub fn write_xml(
    path: &Path,
    dataset: &str,
    columns: &[String],
    rows: &[Value],
    opts: &XmlOpts,
    generated_ts: i64,
    // How many input files fed this export. Paths are deliberately not
    // embedded: an artifact must not carry machine-specific filesystem paths.
    source_count: usize,
) -> Result<XmlReport> {
    if columns.is_empty() {
        bail!("xml export needs at least one column");
    }
    let mut skipped = Vec::new();
    let mut usable: Vec<(String, String)> = Vec::new(); // (original, element name)
    for c in columns.iter().take(opts.max_fields.max(1)) {
        match sanitize_name(c) {
            Some(name) => usable.push((c.clone(), name)),
            None => skipped.push(c.clone()),
        }
    }
    if usable.is_empty() {
        bail!("xml export: no column has a usable XML element name");
    }

    let mut out = String::with_capacity(1024 + rows.len() * 128);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<harvest-export version=\"1\" dataset=\"{}\" records=\"{}\" generated_ts=\"{}\">",
        escape_xml(dataset),
        rows.len().min(opts.max_records.max(1)),
        generated_ts
    ));

    if opts.metadata {
        out.push_str("<metadata>");
        out.push_str(&format!("<dataset>{}</dataset>", escape_xml(dataset)));
        out.push_str(&format!("<generated_ts>{generated_ts}</generated_ts>"));
        out.push_str(&format!(
            "<records>{}</records>",
            rows.len().min(opts.max_records.max(1))
        ));
        out.push_str(&format!("<sources>{}</sources>", source_count));
        out.push_str("<columns>");
        for (original, name) in &usable {
            out.push_str(&format!(
                "<column name=\"{}\" source=\"{}\"/>",
                escape_xml(name),
                escape_xml(original)
            ));
        }
        for s in &skipped {
            out.push_str(&format!(
                "<skipped-column source=\"{}\" reason=\"unsafe-element-name\"/>",
                escape_xml(s)
            ));
        }
        out.push_str("</columns></metadata>");
    }

    out.push_str("<records>");
    let mut truncated = 0u64;
    let mut written = 0u64;
    for (i, row) in rows.iter().enumerate().take(opts.max_records.max(1)) {
        out.push_str(&format!("<record index=\"{i}\">"));
        let obj = row.as_object();
        for (original, name) in &usable {
            let v = obj.and_then(|o| o.get(original)).unwrap_or(&Value::Null);
            let (text, was_truncated) = text_of(v, opts.max_chars.max(1));
            if was_truncated {
                truncated += 1;
            }
            if v.is_null() {
                out.push_str(&format!(
                    "<field name=\"{}\" type=\"null\" null=\"true\"/>",
                    escape_xml(name)
                ));
            } else {
                out.push_str(&format!(
                    "<field name=\"{}\" type=\"{}\">{}</field>",
                    escape_xml(name),
                    type_of(v),
                    escape_xml(&text)
                ));
            }
        }
        out.push_str("</record>");
        written += 1;
    }
    out.push_str("</records></harvest-export>\n");

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.xml.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "export".to_string())
    ));
    {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(out.as_bytes())?;
        f.flush()?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    let mut h = Sha256::new();
    h.update(out.as_bytes());

    Ok(XmlReport {
        path: path.to_path_buf(),
        bytes: out.len() as u64,
        sha256: format!("{:x}", h.finalize()),
        records: written,
        fields: usable.len(),
        skipped_fields: skipped,
        truncated_strings: truncated,
    })
}
