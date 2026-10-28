//! Typed extraction: CSS selector rules -> serde_json values, plus a
//! content fingerprint (normalized-text sha256) used for dedup.

use serde_json::{Map, Value};

use crate::config::FieldCfg;
use scraper::{ElementRef, Html, Selector};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct Extractor {
    fields: Vec<(String, Compiled)>,
}

struct Compiled {
    selector: Selector,
    attr: Option<String>,
    all: bool,
}

impl Extractor {
    pub fn compile(rules: &BTreeMap<String, FieldCfg>) -> anyhow::Result<Self> {
        let mut fields = Vec::new();
        for (name, rule) in rules {
            let selector = Selector::parse(&rule.css).map_err(|_| {
                anyhow::anyhow!("invalid selector `{}` for field `{name}`", rule.css)
            })?;
            fields.push((
                name.clone(),
                Compiled {
                    selector,
                    attr: rule.attr.clone(),
                    all: rule.all,
                },
            ));
        }
        Ok(Self { fields })
    }

    pub fn extract_bounded(&self, doc: &Html) -> anyhow::Result<Map<String, Value>> {
        if self.fields.len() > 1024 {
            anyhow::bail!("extractor exceeds 1024 fields");
        }
        let mut out = Map::new();
        let mut total = 0usize;
        for (name, c) in &self.fields {
            let mut vals = Vec::new();
            for el in doc.select(&c.selector) {
                if vals.len() >= 4096 {
                    anyhow::bail!("field exceeds 4096 matches; use a narrower selector");
                }
                let mut raw = String::new();
                if let Some(attr) = &c.attr {
                    if let Some(v) = el.value().attr(attr) {
                        if v.len() > 1024 * 1024 {
                            anyhow::bail!("extracted field exceeds 1 MiB");
                        }
                        raw.push_str(v.trim());
                    }
                } else {
                    for text in el.text() {
                        if raw.len().saturating_add(text.len()) > 1024 * 1024 {
                            anyhow::bail!("extracted field exceeds 1 MiB");
                        }
                        raw.push_str(text);
                    }
                    raw = collapse_ws(&raw);
                }
                total = total.saturating_add(raw.len() + name.len());
                if total > 8 * 1024 * 1024 {
                    anyhow::bail!("extracted record exceeds 8 MiB");
                }
                if !raw.is_empty() {
                    vals.push(Value::String(raw));
                }
                if !c.all {
                    break;
                }
            }
            out.insert(
                name.clone(),
                if vals.is_empty() {
                    Value::Null
                } else if c.all {
                    Value::Array(vals)
                } else {
                    vals.remove(0)
                },
            );
        }
        Ok(out)
    }

    pub fn extract(&self, doc: &Html) -> Map<String, Value> {
        let mut out = Map::new();
        for (name, c) in &self.fields {
            let mut vals: Vec<String> = Vec::new();
            for el in doc.select(&c.selector) {
                if let Some(a) = &c.attr {
                    if let Some(v) = el.value().attr(a) {
                        vals.push(v.trim().to_string());
                    }
                } else {
                    let raw: String = el.text().collect();
                    let t = collapse_ws(&raw);
                    if !t.is_empty() {
                        vals.push(t);
                    }
                }
                if !c.all {
                    break;
                }
            }
            let v = if vals.is_empty() {
                Value::Null
            } else if c.all {
                Value::Array(vals.into_iter().map(Value::String).collect())
            } else {
                Value::String(vals.into_iter().next().unwrap())
            };
            out.insert(name.clone(), v);
        }
        out
    }
}

/// Collapse all Unicode whitespace (incl. NBSP) runs into single spaces.
pub fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Normalize visible text (collapse whitespace) then hash. Two identical
/// product pages served from different URLs dedup to one fingerprint.
pub fn fingerprint(html: &str) -> String {
    let doc = Html::parse_document(html);
    let body: Option<ElementRef> = doc
        .select(&Selector::parse("body").unwrap())
        .next()
        .or_else(|| doc.select(&Selector::parse("html").unwrap()).next());
    let text: String = match body {
        Some(el) => collapse_ws(&el.text().collect::<String>()),
        None => collapse_ws(html),
    };
    let mut h = Sha256::new();
    h.update(text.as_bytes());
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Discover same-host links for crawling.
pub fn links(doc: &Html, selector: &Selector) -> Vec<String> {
    doc.select(selector)
        .filter_map(|el| el.value().attr("href").map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// JSONPath-lite: enough of the syntax for API harvesting, fully deterministic.
//   $ .key [n] [*]     e.g.  $.data.items[*].name   $.links.next[0].href
// Keys are [A-Za-z0-9_-]+ ; quoted/regex/filter expressions are rejected at
// compile time rather than silently ignored — a typo'd selector must fail
// validation, not quietly return nulls in a production pipeline.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum PathStep {
    Key(String),
    Index(usize),
    Wildcard,
}

/// Parse and validate a path; used by `Config::validate` so bad selectors
/// fail before any network request is made.
pub fn compile_path(raw: &str) -> anyhow::Result<Vec<PathStep>> {
    let s = raw.trim();
    let mut chars = s.char_indices().peekable();
    let mut steps = Vec::new();
    match chars.next() {
        Some((_, '$')) => {}
        _ => anyhow::bail!("path must start with `$`: `{raw}`"),
    }
    while let Some((_, c)) = chars.next() {
        match c {
            '.' => {
                let mut key = String::new();
                while let Some(&(i, ch)) = chars.peek() {
                    if ch == '.' || ch == '[' {
                        break;
                    }
                    key.push(ch);
                    chars.next();
                    let _ = i;
                }
                if key.is_empty()
                    || !key
                        .chars()
                        .all(|k| k.is_ascii_alphanumeric() || matches!(k, '_' | '-'))
                {
                    anyhow::bail!("bad key `{key}` in `{raw}` (allowed: A-Za-z0-9 _ -)");
                }
                steps.push(PathStep::Key(key));
            }
            '[' => {
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some((_, ']')) => break,
                        Some((_, ch)) => inner.push(ch),
                        None => anyhow::bail!("unterminated `[` in `{raw}`"),
                    }
                }
                if inner == "*" {
                    steps.push(PathStep::Wildcard);
                } else if let Ok(n) = inner.parse::<usize>() {
                    steps.push(PathStep::Index(n));
                } else {
                    anyhow::bail!("`[{inner}]` must be `[*]` or an index in `{raw}`");
                }
            }
            ch => anyhow::bail!("unexpected char `{ch}` in `{raw}`"),
        }
    }
    if steps.is_empty() {
        anyhow::bail!("path selects nothing (just `$`): `{raw}`");
    }
    Ok(steps)
}

/// Evaluate compiled steps; returns every match (wildcards expand).
pub fn query_path<'a>(root: &'a Value, steps: &[PathStep]) -> Vec<&'a Value> {
    let mut cur = vec![root];
    for st in steps {
        let mut next = Vec::new();
        for v in cur {
            match (st, v) {
                (PathStep::Key(k), Value::Object(m)) => {
                    if let Some(x) = m.get(k) {
                        next.push(x);
                    }
                }
                (PathStep::Index(i), Value::Array(a)) => {
                    if let Some(x) = a.get(*i) {
                        next.push(x);
                    }
                }
                (PathStep::Wildcard, Value::Array(a)) => next.extend(a.iter()),
                (PathStep::Wildcard, Value::Object(m)) => next.extend(m.values()),
                _ => {}
            }
        }
        cur = next;
        if cur.is_empty() {
            break;
        }
    }
    cur
}

#[derive(Debug)]
pub struct JsonField {
    steps: Vec<PathStep>,
    all: bool,
}

#[derive(Debug)]
pub struct JsonExtractor {
    fields: Vec<(String, JsonField)>,
}

impl JsonExtractor {
    pub fn compile(rules: &BTreeMap<String, FieldCfg>) -> anyhow::Result<Self> {
        let mut fields = Vec::new();
        for (name, rule) in rules {
            let raw = rule
                .path
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("field `{name}` has no `path` (json mode)"))?;
            fields.push((
                name.clone(),
                JsonField {
                    steps: compile_path(raw)?,
                    all: rule.all,
                },
            ));
        }
        Ok(Self { fields })
    }

    /// Native JSON types are preserved (numbers stay numbers, bools stay
    /// bools) — unlike CSS text extraction, no string coercion.
    pub fn extract_bounded(&self, doc: &Value) -> anyhow::Result<Map<String, Value>> {
        let mut budget = CountBytes(8 * 1024 * 1024);
        let mut out = Map::new();
        for (name, field) in &self.fields {
            let hits = query_path(doc, &field.steps);
            if hits.len() > 4096 {
                anyhow::bail!("JSON field exceeds 4096 matches");
            }
            let picked: Vec<&Value> = if field.all {
                hits
            } else {
                hits.into_iter().take(1).collect()
            };
            for v in &picked {
                serde_json::to_writer(&mut budget, v)
                    .map_err(|_| anyhow::anyhow!("JSON record exceeds 8 MiB"))?;
            }
            let v = if picked.is_empty() {
                Value::Null
            } else if field.all {
                Value::Array(picked.into_iter().cloned().collect())
            } else {
                picked[0].clone()
            };
            out.insert(name.clone(), v);
        }
        Ok(out)
    }
    pub fn extract(&self, doc: &Value) -> Map<String, Value> {
        let mut out = Map::new();
        for (name, f) in &self.fields {
            let hits = query_path(doc, &f.steps);
            let v = if hits.is_empty() {
                Value::Null
            } else if f.all {
                Value::Array(hits.into_iter().cloned().collect())
            } else {
                hits[0].clone()
            };
            out.insert(name.clone(), v);
        }
        out
    }
}

struct CountBytes(usize);
impl std::io::Write for CountBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(std::io::Error::other("record byte budget exceeded"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(crate) fn guard_html(body: &str) -> anyhow::Result<()> {
    if body.bytes().filter(|b| *b == b'<').take(100_001).count() > 100_000 {
        anyhow::bail!("HTML exceeds the 100000-tag complexity cap");
    }
    Ok(())
}
pub(crate) fn fingerprint_json(body: &str) -> anyhow::Result<String> {
    let value: Value = serde_json::from_str(body)?;
    let bytes = serde_json::to_vec(&value)?;
    let hash = Sha256::digest(bytes);
    Ok(hash.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn links_bounded(doc: &Html, selector: &Selector, cap: usize) -> Vec<String> {
    doc.select(selector)
        .filter_map(|el| el.value().attr("href"))
        .filter(|href| href.len() <= 8192)
        .take(cap.min(100_000))
        .map(str::to_string)
        .collect()
}
