//! Declared record schemas: the validation/correction layer between the
//! transform pipeline and the sink — plus the quarantine lane and the
//! draft-07 export used for data-contract review.
//!
//! Modes, deliberately boring:
//! * `off` — pure observation: type-drift counting only (metrics/debug).
//! * `coerce` — repair what is mechanically repairable (text "42" to int,
//!   bool to "true"), counting every repair as a schema drift.
//! * `strict` — any violation (transform failure, coercion failure, or a
//!   shape mismatch coercion cannot fix) sends the whole record to the
//!   JSONL quarantine with its reasons, and the job still succeeds:
//!   quarantining is a data outcome, not a fetch failure, so one bad page
//!   never burns retries.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::config::{SchemaCfg, TargetCfg};
use crate::transform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enforce {
    Off,
    Coerce,
    Strict,
}

/// A declared field type. `text` is total (everything stringifies);
/// `number`/`int`/`bool`/`date` accept matching JSON types plus text that
/// the transform parsers accept; `list` also splits plain text on commas;
/// `json` is anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredType {
    Text,
    Number,
    Int,
    Bool,
    Date,
    List,
    Json,
}

impl DeclaredType {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => Self::Text,
            "number" => Self::Number,
            "int" => Self::Int,
            "bool" => Self::Bool,
            "date" => Self::Date,
            "list" => Self::List,
            "json" => Self::Json,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Date => "date",
            Self::List => "list",
            Self::Json => "json",
        }
    }
}

/// Compiled per-target schema.
#[derive(Debug)]
pub struct Schema {
    pub enforce: Enforce,
    pub fields: Vec<(String, DeclaredType)>,
    pub quarantine: Option<PathBuf>,
}

/// Outcome of checking (and possibly repairing) one record.
#[derive(Debug, Default)]
pub struct ApplyReport {
    /// Fields coerced to their declared type (counted as drift in `coerce`).
    pub coerced: Vec<String>,
    /// Field-level failures: (field, reason). Non-empty + `strict` ⇒ quarantine.
    pub failures: Vec<(String, String)>,
}

impl Schema {
    pub fn compile(t: &TargetCfg) -> anyhow::Result<Option<Self>> {
        if t.schema.enforce == "off" && t.schema.fields.is_empty() {
            return Ok(None);
        }
        let enforce = match t.schema.enforce.as_str() {
            "off" => Enforce::Off,
            "coerce" => Enforce::Coerce,
            "strict" => Enforce::Strict,
            _ => anyhow::bail!("target {}: schema.enforce invalid", t.id),
        };
        let mut fields = Vec::with_capacity(t.schema.fields.len());
        for (name, ty) in &t.schema.fields {
            let Some(dt) = DeclaredType::parse(ty) else {
                anyhow::bail!(
                    "target {}: unknown schema type `{ty}` for field `{name}`",
                    t.id
                );
            };
            fields.push((name.clone(), dt));
        }
        if enforce == Enforce::Strict && t.schema.quarantine_path.is_none() {
            anyhow::bail!(
                "target {}: schema.enforce=strict requires schema.quarantine_path \
                 (otherwise quarantined records would silently vanish)",
                t.id
            );
        }
        Ok(Some(Self {
            enforce,
            fields,
            quarantine: t.schema.quarantine_path.clone(),
        }))
    }

    /// Check + (per mode) repair fields in place. `transform_fails` are
    /// `(field, reason)` pairs from the transform pass — they stay failures
    /// regardless of coercion luck.
    pub fn apply(
        &self,
        fields: &mut Map<String, Value>,
        transform_fails: Vec<(String, String)>,
    ) -> ApplyReport {
        let mut report = ApplyReport {
            coerced: Vec::new(),
            failures: transform_fails,
        };
        for (name, ty) in &self.fields {
            let Some(current) = fields.get(name) else {
                continue; // absence is the extractor's business (required_fields), not the schema's
            };
            let value = current.clone();
            match check_one(*ty, value.clone()) {
                Check::Ok(next) => {
                    if next != value {
                        report.coerced.push(name.clone());
                        if self.enforce != Enforce::Off {
                            fields.insert(name.clone(), next);
                        }
                    }
                }
                Check::Bad(why) => {
                    if self.enforce == Enforce::Coerce {
                        // A coercion-mode field that cannot be coerced is
                        // still a violation — but Coerce mode has no
                        // quarantine lane: surface it as a repair failure
                        // without changing the record.
                        report.failures.push((name.clone(), why));
                    } else {
                        report.failures.push((name.clone(), why));
                    }
                }
            }
        }
        report
    }

    /// Append the record to the quarantine JSONL. Returns the path used.
    pub fn quarantine(
        &self,
        url: &str,
        depth: usize,
        fields: &Map<String, Value>,
        reasons: &[(String, String)],
    ) -> anyhow::Result<Option<PathBuf>> {
        let Some(path) = &self.quarantine else {
            return Ok(None);
        };
        let mut entry = Map::new();
        entry.insert(
            "url".into(),
            Value::String(crate::config::redact_credentials(url)),
        );
        entry.insert("depth".into(), Value::Number(depth.into()));
        entry.insert("ts".into(), Value::String(now_rfc3339()));
        entry.insert(
            "reasons".into(),
            Value::Array(
                reasons
                    .iter()
                    .map(|(f, r)| Value::String(format!("{f}: {r}")))
                    .collect(),
            ),
        );
        entry.insert("fields".into(), Value::Object(fields.clone()));
        let line = serde_json::to_string(&Value::Object(entry))?;
        let mut file = crate::fsutil::open_append(path)
            .map_err(|e| anyhow::anyhow!("open quarantine {}: {e}", path.display()))?;
        writeln!(file, "{line}")?;
        Ok(Some(path.clone()))
    }
}

enum Check {
    Ok(Value),
    Bad(String),
}

fn check_one(ty: DeclaredType, v: Value) -> Check {
    use DeclaredType as D;
    match ty {
        D::Json => Check::Ok(v),
        D::Text => match &v {
            Value::String(_) => Check::Ok(v),
            Value::Null => Check::Ok(Value::String(String::new())),
            Value::Bool(b) => Check::Ok(Value::String(b.to_string())),
            Value::Number(n) => Check::Ok(Value::String(n.to_string())),
            Value::Array(items) => Check::Ok(Value::String(
                items.iter().map(text_of).collect::<Vec<_>>().join(", "),
            )),
            Value::Object(_) => Check::Bad("text cannot losslessly flatten an object".into()),
        },
        D::Number | D::Int => match &v {
            Value::Number(n) => {
                if ty == D::Int && n.as_i64().is_none() && n.as_u64().is_none() {
                    // float in an int field: round, that's the coerce story
                    if let Some(f) = n.as_f64() {
                        let r = f.round_ties_even();
                        if (r.abs() - f.abs()).abs() < 1e-9 || ty == D::Number {
                            Check::Ok(v.clone())
                        } else {
                            Check::Ok(Value::Number((r as i64).into()))
                        }
                    } else {
                        Check::Ok(v.clone())
                    }
                } else {
                    Check::Ok(v.clone())
                }
            }
            Value::String(s) => match transform::parse_number(s, ty == D::Int) {
                Ok(n) => Check::Ok(n),
                Err(why) => Check::Bad(why),
            },
            other => Check::Bad(format!(
                "{} needs a number, got {}",
                ty.as_str(),
                kind(other)
            )),
        },
        D::Bool => match &v {
            Value::Bool(_) => Check::Ok(v),
            Value::String(s) => {
                let t = transform::collapse_ws(s).to_lowercase();
                match t.as_str() {
                    "true" | "yes" | "1" => Check::Ok(Value::Bool(true)),
                    "false" | "no" | "0" => Check::Ok(Value::Bool(false)),
                    _ => Check::Bad(format!("bool needs a boolean, got `{t}`")),
                }
            }
            Value::Number(n) => match n.as_i64() {
                Some(0) => Check::Ok(Value::Bool(false)),
                Some(1) => Check::Ok(Value::Bool(true)),
                _ => Check::Bad("bool needs 0/1".into()),
            },
            other => Check::Bad(format!("bool needs a boolean, got {}", kind(other))),
        },
        D::Date => match &v {
            Value::String(s) => match transform::parse_date(s, false) {
                Ok(norm) => Check::Ok(norm),
                Err(why) => Check::Bad(why),
            },
            Value::Number(n) if n.as_i64().is_some() => {
                let s = n.to_string();
                match transform::parse_date(&s, false) {
                    Ok(norm) => Check::Ok(norm),
                    Err(why) => Check::Bad(why),
                }
            }
            other => Check::Bad(format!(
                "date needs text or epoch seconds, got {}",
                kind(other)
            )),
        },
        D::List => match v {
            Value::Array(_) => Check::Ok(v),
            Value::String(s) => Check::Ok(Value::Array(
                s.split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(|p| Value::String(p.to_string()))
                    .collect(),
            )),
            scalar => Check::Ok(Value::Array(vec![scalar])),
        },
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "text",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}

fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    crate::transform::epoch_rfc3339(secs)
}

/// Draft-07 for one target: types from the declared schema when present,
/// otherwise from the field's transform chain (the inference that makes
/// `extract` rules self-documenting). `harvest schema` prints this.
pub fn json_schema_for(target: &TargetCfg) -> Value {
    let mut props = Map::new();
    let mut required: Vec<Value> = Vec::new();
    for (name, rule) in &target.extract {
        let mut prop = Map::new();
        let declared = target
            .schema
            .fields
            .get(name)
            .and_then(|t| DeclaredType::parse(t));
        let (ty, fmt) = match declared {
            Some(dt) => match dt {
                DeclaredType::Text => ("string", None),
                DeclaredType::Number => ("number", None),
                DeclaredType::Int => ("integer", None),
                DeclaredType::Bool => ("boolean", None),
                DeclaredType::Date => ("string", Some("date-time")),
                DeclaredType::List => ("array", None),
                DeclaredType::Json => ("object", None),
            },
            None => inferred_from_transforms(rule),
        };
        prop.insert("type".into(), Value::String(ty.into()));
        if let Some(f) = fmt {
            prop.insert("format".into(), Value::String(f.into()));
        }
        if rule.attr.is_some() || rule.path.is_some() || rule.all {
            let mut notes: Vec<String> = Vec::new();
            if let Some(a) = &rule.attr {
                notes.push(format!("attr: {a}"));
            }
            if let Some(p) = &rule.path {
                notes.push(format!("jsonpath: {p}"));
            }
            if rule.all {
                notes.push("all matches".into());
            }
            prop.insert("harvest:source".into(), Value::String(notes.join(", ")));
        }
        if !rule.transforms.is_empty() {
            prop.insert(
                "harvest:transforms".into(),
                Value::Array(
                    rule.transforms
                        .iter()
                        .map(|t| Value::String(op_name(t)))
                        .collect(),
                ),
            );
        }
        props.insert(name.clone(), Value::Object(prop));
        required.push(Value::String(name.clone()));
    }
    let mut schema = Map::new();
    schema.insert(
        "$schema".into(),
        Value::String("http://json-schema.org/draft-07/schema#".into()),
    );
    schema.insert(
        "title".into(),
        Value::String(format!("harvest:{}", target.id)),
    );
    schema.insert("type".into(), Value::String("object".into()));
    schema.insert("properties".into(), Value::Object(props));
    schema.insert("required".into(), Value::Array(required));
    schema.insert("additionalProperties".into(), Value::Bool(false));
    Value::Object(schema)
}

fn inferred_from_transforms(
    rule: &crate::config::FieldCfg,
) -> (&'static str, Option<&'static str>) {
    use crate::config::TransformCfg as T;
    // Last typing transform wins; `all: true` makes everything an array.
    let mut ty = "string";
    let mut fmt = None;
    for t in &rule.transforms {
        match t {
            T::ParseNumber { as_int: true } => ty = "integer",
            T::ParseNumber { .. } | T::RoundTo { .. } | T::Units { .. } => ty = "number",
            T::ToBool { .. } => ty = "boolean",
            T::ParseDate { output } => {
                if output == "unix" {
                    ty = "integer";
                } else {
                    ty = "string";
                    fmt = Some("date-time");
                }
            }
            T::Split { .. } => ty = "array",
            T::Unique | T::Sort | T::Take { .. } | T::Drop { .. } => ty = "array",
            T::Join { .. } => ty = "string",
            _ => {}
        }
    }
    if rule.all && !matches!(ty, "array") {
        ty = "array";
    }
    (ty, fmt)
}

fn op_name(cfg: &crate::config::TransformCfg) -> String {
    use crate::config::TransformCfg as T;
    match cfg {
        T::Trim => "trim".into(),
        T::Lowercase => "lowercase".into(),
        T::Uppercase => "uppercase".into(),
        T::CollapseWhitespace => "collapse_whitespace".into(),
        T::RegexReplace { pattern, .. } => format!("regex_replace({pattern})"),
        T::RegexExtract { pattern, .. } => format!("regex_extract({pattern})"),
        T::StripTags => "strip_tags".into(),
        T::MaskPii { strategy, kinds } => {
            let k = if kinds.is_empty() {
                "all".to_string()
            } else {
                kinds.join("|")
            };
            format!("mask_pii({k},{strategy:?})")
        }
        T::ParseNumber { as_int } => format!("parse_number(as_int={as_int})"),
        T::RoundTo { decimals } => format!("round_to({decimals})"),
        T::Units { to } => format!("units(to={to})"),
        T::ParseDate { output } => format!("parse_date(output={output})"),
        T::ToBool { .. } => "to_bool".into(),
        T::Split { sep, .. } => format!("split(sep={sep:?})"),
        T::Join { sep } => format!("join(sep={sep:?})"),
        T::Unique => "unique".into(),
        T::Sort => "sort".into(),
        T::Take { count } => format!("take({count})"),
        T::Drop { count } => format!("drop({count})"),
        T::Truncate { max_chars } => format!("truncate({max_chars})"),
        T::ReplaceEmpty { with } => format!("replace_empty({with:?})"),
        T::Prefix { with } => format!("prefix({with:?})"),
        T::Suffix { with } => format!("suffix({with:?})"),
    }
}

/// Quarantine writer for `coerce`-mode logging parity (append-only; one fs
/// open per record is acceptable at quarantining rates — failures are).
pub fn append_line(path: &Path, line: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// SchemaCfg helper for Default-driven code paths.
impl SchemaCfg {
    pub fn is_enforcing(&self) -> bool {
        self.enforce != "off"
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests;
