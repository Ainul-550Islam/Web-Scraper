//! Value transforms between extraction and the record: the pipeline that
//! turns scraped text into typed, normalized, redaction-safe data.
//!
//! Design rules, matching the engine's philosophy:
//! * Every step is a pure `Value -> Result<Value, String>` — no IO, no
//!   clock, no locale guessing beyond what the config names explicitly.
//! * Steps map over arrays element-wise (string ops) so `all: true` fields
//!   behave like their scalar twins; list ops are array-only by design.
//! * A failing step does NOT destroy data: the original value is kept and
//!   the failure is reported — the target's schema policy decides whether
//!   that quarantines the record or just counts a drift.
//! * PII masking is collection-time redaction (what you *store*), using
//!   validated patterns (Luhn for card numbers, octet-checked IPv4) so the
//!   redactor cannot be tricked by near-miss noise into mangling prices.

use std::collections::BTreeMap;

use regex::Regex;
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

use crate::config::{FieldCfg, MaskStrategy, TargetCfg, TransformCfg};
use crate::schema::{Enforce, Schema};

/// One compiled transform step. Regexes compile once per run (and once
/// again inside `Config::validate`), never per record.
#[derive(Debug)]
pub enum Transform {
    Trim,
    Lowercase,
    Uppercase,
    CollapseWhitespace,
    RegexReplace {
        re: Regex,
        with: String,
    },
    RegexExtract {
        re: Regex,
        group: usize,
    },
    StripTags,
    MaskPii {
        email: bool,
        phone: bool,
        card: bool,
        ipv4: bool,
        strategy: MaskStrategy,
    },
    ParseNumber {
        as_int: bool,
    },
    RoundTo {
        decimals: u32,
    },
    Units {
        to: UnitTarget,
    },
    ParseDate {
        unix_out: bool,
    },
    ToBool {
        t: Vec<String>,
        f: Vec<String>,
    },
    Split {
        sep: String,
        trim: bool,
        max: usize,
    },
    Join {
        sep: String,
    },
    Unique,
    Sort,
    Take {
        count: usize,
    },
    Drop {
        count: usize,
    },
    Truncate {
        max_chars: usize,
    },
    ReplaceEmpty {
        with: String,
    },
    Prefix {
        with: String,
    },
    Suffix {
        with: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum UnitTarget {
    Bytes,
    Seconds,
    Percent,
}

/// A transform failure: which field, which step, why.
#[derive(Debug, Clone)]
pub struct FieldIssue {
    pub field: String,
    pub step: &'static str,
    pub reason: String,
}

impl FieldIssue {
    pub fn to_string_line(&self) -> String {
        format!("{}: {}/{}", self.field, self.step, self.reason)
    }
}

/// Compiled per-target field chains.
#[derive(Debug, Default)]
pub struct Pipeline {
    chains: BTreeMap<String, Vec<Transform>>,
}

impl Pipeline {
    /// Compile every field's chain. Errors name the offending field so the
    /// operator gets "field `href`: bad pattern …" instead of a panic.
    pub fn compile(rules: &BTreeMap<String, FieldCfg>) -> anyhow::Result<Self> {
        let mut chains = BTreeMap::new();
        for (name, rule) in rules {
            if rule.transforms.is_empty() {
                continue;
            }
            let mut v = Vec::with_capacity(rule.transforms.len());
            for t in &rule.transforms {
                v.push(compile_step(name, t)?);
            }
            chains.insert(name.clone(), v);
        }
        Ok(Self { chains })
    }

    /// True when no field has any transform — the engine then skips the
    /// entire pass (zero allocation on the hot path for legacy configs).
    pub fn is_identity(&self) -> bool {
        self.chains.is_empty()
    }

    pub fn has_field(&self, field: &str) -> bool {
        self.chains.contains_key(field)
    }

    /// Apply in place; returns issues per failed step. On failure the field
    /// keeps its PRE-CHAIN value — a half-transformed field is the worst of
    /// both worlds, so chains are atomic per field.
    pub fn apply(&self, fields: &mut Map<String, Value>) -> Vec<FieldIssue> {
        let mut issues = Vec::new();
        for (field, chain) in &self.chains {
            let Some(original) = fields.get(field).cloned() else {
                continue;
            };
            let mut value = original.clone();
            let mut failed: Option<(&'static str, String)> = None;
            for step in chain {
                match step.run(value.clone()) {
                    Ok(next) => value = next,
                    Err(reason) => {
                        failed = Some((step.name(), reason));
                        break;
                    }
                }
            }
            match failed {
                Some((step, reason)) => issues.push(FieldIssue {
                    field: field.clone(),
                    step,
                    reason,
                }),
                None => {
                    fields.insert(field.clone(), value);
                }
            }
        }
        issues
    }
}

impl Transform {
    pub fn name(&self) -> &'static str {
        match self {
            Transform::Trim => "trim",
            Transform::Lowercase => "lowercase",
            Transform::Uppercase => "uppercase",
            Transform::CollapseWhitespace => "collapse_whitespace",
            Transform::RegexReplace { .. } => "regex_replace",
            Transform::RegexExtract { .. } => "regex_extract",
            Transform::StripTags => "strip_tags",
            Transform::MaskPii { .. } => "mask_pii",
            Transform::ParseNumber { .. } => "parse_number",
            Transform::RoundTo { .. } => "round_to",
            Transform::Units { .. } => "units",
            Transform::ParseDate { .. } => "parse_date",
            Transform::ToBool { .. } => "to_bool",
            Transform::Split { .. } => "split",
            Transform::Join { .. } => "join",
            Transform::Unique => "unique",
            Transform::Sort => "sort",
            Transform::Take { .. } => "take",
            Transform::Drop { .. } => "drop",
            Transform::Truncate { .. } => "truncate",
            Transform::ReplaceEmpty { .. } => "replace_empty",
            Transform::Prefix { .. } => "prefix",
            Transform::Suffix { .. } => "suffix",
        }
    }

    fn run(&self, v: Value) -> Result<Value, String> {
        use Transform as T;
        match self {
            // ---- string steps (element-wise over arrays) ----
            T::Trim => map_text(v, |s| Ok(s.trim().to_string())),
            T::Lowercase => map_text(v, |s| Ok(s.to_lowercase())),
            T::Uppercase => map_text(v, |s| Ok(s.to_uppercase())),
            T::CollapseWhitespace => map_text(v, |s| Ok(collapse_ws(s))),
            T::RegexReplace { re, with } => {
                map_text(v, |s| Ok(re.replace_all(s, with.as_str()).into_owned()))
            }
            T::RegexExtract { re, group } => map_text(v, |s| {
                let Some(caps) = re.captures(s) else {
                    return Err(format!("no match for /{re}/ in `{}`", trunc(s, 80)));
                };
                let g = caps
                    .get(*group)
                    .ok_or_else(|| format!("group {group} not in pattern"))?;
                Ok(g.as_str().to_string())
            }),
            T::StripTags => map_text(v, |s| Ok(dehtml(s))),
            T::MaskPii {
                email,
                phone,
                card,
                ipv4,
                strategy,
            } => map_text(v, |s| {
                Ok(mask_pii(s, *email, *phone, *card, *ipv4, *strategy))
            }),
            T::ParseNumber { as_int } => map_number_aware(v, *as_int, |s| parse_number(s, *as_int)),
            T::RoundTo { decimals } => match v {
                // numbers round from their exact value; text goes through
                // parse_number's documented thousands rules first.
                Value::Number(ref n) => round_number(n, *decimals),
                Value::Array(items) => items
                    .into_iter()
                    .map(|it| match it {
                        Value::Number(n) => round_number(&n, *decimals),
                        other => round_to(&text_of(&other), *decimals),
                    })
                    .collect(),
                other => round_to(&text_of(&other), *decimals),
            },
            T::Units { to } => map_scalar(v, |s| convert_units(&s, *to)),
            T::ParseDate { unix_out } => map_scalar(v, |s| parse_date(&s, *unix_out)),
            T::ToBool { t, f } => map_scalar(v, |s| {
                let norm = collapse_ws(&s).to_lowercase();
                if t.iter().any(|x| x.to_lowercase() == norm) {
                    Ok(Value::Bool(true))
                } else if f.iter().any(|x| x.to_lowercase() == norm) {
                    Ok(Value::Bool(false))
                } else {
                    Err(format!(
                        "`{}` is not a configured boolean",
                        trunc(&norm, 60)
                    ))
                }
            }),
            // ---- text -> list / list -> text ----
            T::Split { sep, trim, max } => match &v {
                Value::String(_) => {
                    let s = v.as_str().unwrap_or_default().to_string();
                    let parts: Vec<Value> = if *max > 0 {
                        s.splitn(*max, sep.as_str()).collect::<Vec<_>>()
                    } else {
                        s.split(sep.as_str()).collect::<Vec<_>>()
                    }
                    .into_iter()
                    .map(|p: &str| {
                        let p = if *trim { p.trim() } else { p };
                        Value::String(p.to_string())
                    })
                    .collect();
                    Ok(Value::Array(parts))
                }
                other => Err(format!("split needs text, got {}", kind_of(other))),
            },
            T::Join { sep } => match v {
                Value::Array(items) => {
                    let parts: Vec<String> = items.iter().map(text_of).collect();
                    Ok(Value::String(parts.join(sep)))
                }
                other => Err(format!("join needs a list, got {}", kind_of(&other))),
            },
            // ---- list steps ----
            T::Unique => match v {
                Value::Array(items) => {
                    let mut out: Vec<Value> = Vec::with_capacity(items.len());
                    for it in items {
                        if !out.contains(&it) {
                            out.push(it);
                        }
                    }
                    Ok(Value::Array(out))
                }
                other => Err(format!("unique needs a list, got {}", kind_of(&other))),
            },
            T::Sort => match v {
                Value::Array(mut items) => {
                    items.sort_by_key(text_of);
                    Ok(Value::Array(items))
                }
                other => Err(format!("sort needs a list, got {}", kind_of(&other))),
            },
            T::Take { count } => slice_op(v, |items| items[..items.len().min(*count)].to_vec()),
            T::Drop { count } => slice_op(v, |items| items[items.len().min(*count)..].to_vec()),
            T::Truncate { max_chars } => map_text(v, |s| {
                let n = s.chars().count();
                if n <= *max_chars {
                    Ok(s.to_string())
                } else {
                    let cut: String = s.chars().take(*max_chars).collect();
                    Ok(format!("{cut}…"))
                }
            }),
            // ---- conditional text steps ----
            T::ReplaceEmpty { with } => map_text(v, |s| {
                if s.trim().is_empty() {
                    Ok(with.clone())
                } else {
                    Ok(s.to_string())
                }
            }),
            T::Prefix { with } => map_text(v, |s| Ok(format!("{with}{s}"))),
            T::Suffix { with } => map_text(v, |s| Ok(format!("{s}{with}"))),
        }
    }
}

fn slice_op(v: Value, f: impl Fn(&[Value]) -> Vec<Value>) -> Result<Value, String> {
    match v {
        Value::Array(items) => Ok(Value::Array(f(&items))),
        other => Err(format!("list op needs a list, got {}", kind_of(&other))),
    }
}

/// String step: applies to text scalars and element-wise on arrays of text;
/// numbers/bools stringify (trim on a number is a no-op, not an error).
fn map_text(v: Value, f: impl Fn(&str) -> Result<String, String>) -> Result<Value, String> {
    match v {
        Value::String(s) => f(&s).map(Value::String),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    Value::String(s) => out.push(Value::String(f(&s)?)),
                    scalar => out.push(Value::String(f(&text_of(&scalar))?)),
                }
            }
            Ok(Value::Array(out))
        }
        scalar => Ok(Value::String(f(&text_of(&scalar))?)),
    }
}

/// Scalar->typed step: number/bool/date converters. Arrays map element-wise.
fn map_scalar(v: Value, f: impl Fn(String) -> Result<Value, String>) -> Result<Value, String> {
    match v {
        Value::Array(items) => items.into_iter().map(|it| f(text_of(&it))).collect(),
        scalar => f(text_of(&scalar)),
    }
}

/// Steps whose input may already be a JSON number: use the exact numeric
/// value instead of stringify-then-reparse — `"1.005"` as TEXT means 1005
/// under the thousands rule, while the number 1.005 must stay itself.
/// `as_int` rounds number inputs on the same half-away curve as text.
fn map_number_aware(
    v: Value,
    as_int: bool,
    from_text: impl Fn(&str) -> Result<Value, String>,
) -> Result<Value, String> {
    let from_f = |f: f64| -> Value {
        let x = if f >= 0.0 {
            (f + 0.5).floor()
        } else {
            (f - 0.5).ceil()
        };
        if x.abs() < i64::MAX as f64 {
            Value::Number((x as i64).into())
        } else {
            Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
    };
    match v {
        Value::Number(n) => {
            if as_int {
                match n.as_f64() {
                    Some(f) => Ok(from_f(f)),
                    None => Ok(Value::Number(n)),
                }
            } else {
                Ok(Value::Number(n))
            }
        }
        Value::Array(items) => items
            .into_iter()
            .map(|it| match it {
                Value::Number(n) => Ok(Value::Number(n)),
                other => from_text(&text_of(&other)),
            })
            .collect(),
        Value::String(s) => from_text(&s),
        other => from_text(&text_of(&other)),
    }
}

fn compile_step(field: &str, cfg: &TransformCfg) -> anyhow::Result<Transform> {
    let bad =
        |e: regex::Error, what: &str| anyhow::anyhow!("field `{field}`: bad {what} pattern: {e}");
    Ok(match cfg {
        TransformCfg::Trim => Transform::Trim,
        TransformCfg::Lowercase => Transform::Lowercase,
        TransformCfg::Uppercase => Transform::Uppercase,
        TransformCfg::CollapseWhitespace => Transform::CollapseWhitespace,
        TransformCfg::RegexReplace {
            pattern,
            replacement,
        } => Transform::RegexReplace {
            re: Regex::new(pattern).map_err(|e| bad(e, "regex_replace"))?,
            with: replacement.clone(),
        },
        TransformCfg::RegexExtract { pattern, group } => Transform::RegexExtract {
            re: Regex::new(pattern).map_err(|e| bad(e, "regex_extract"))?,
            group: *group,
        },
        TransformCfg::StripTags => Transform::StripTags,
        TransformCfg::MaskPii { strategy, kinds } => {
            let all = kinds.is_empty();
            let wants = |k: &str| all || kinds.iter().any(|x| x == k);
            for k in kinds {
                if !matches!(k.as_str(), "email" | "phone" | "card" | "ipv4") {
                    anyhow::bail!(
                        "field `{field}`: unknown mask_pii kind `{k}` \
                         (email|phone|card|ipv4)"
                    );
                }
            }
            Transform::MaskPii {
                email: wants("email"),
                phone: wants("phone"),
                card: wants("card"),
                ipv4: wants("ipv4"),
                strategy: *strategy,
            }
        }
        TransformCfg::ParseNumber { as_int } => Transform::ParseNumber { as_int: *as_int },
        TransformCfg::RoundTo { decimals } => Transform::RoundTo {
            decimals: *decimals,
        },
        TransformCfg::Units { to } => Transform::Units {
            to: match to.as_str() {
                "bytes" => UnitTarget::Bytes,
                "seconds" => UnitTarget::Seconds,
                "percent" => UnitTarget::Percent,
                other => anyhow::bail!(
                    "field `{field}`: units.to must be bytes|seconds|percent (got `{other}`)"
                ),
            },
        },
        TransformCfg::ParseDate { output } => Transform::ParseDate {
            unix_out: match output.as_str() {
                "rfc3339" => false,
                "unix" => true,
                other => anyhow::bail!(
                    "field `{field}`: parse_date.output must be rfc3339|unix (got `{other}`)"
                ),
            },
        },
        TransformCfg::ToBool {
            true_values,
            false_values,
        } => {
            let t = if true_values.is_empty() {
                [
                    "true",
                    "yes",
                    "y",
                    "1",
                    "on",
                    "enabled",
                    "available",
                    "in stock",
                ]
                .map(str::to_string)
                .to_vec()
            } else {
                true_values.clone()
            };
            let f = if false_values.is_empty() {
                [
                    "false",
                    "no",
                    "n",
                    "0",
                    "off",
                    "disabled",
                    "unavailable",
                    "out of stock",
                ]
                .map(str::to_string)
                .to_vec()
            } else {
                false_values.clone()
            };
            for x in t.iter().chain(f.iter()) {
                if x.trim().is_empty() {
                    anyhow::bail!("field `{field}`: to_bool values must not be empty");
                }
            }
            let overlap: Vec<_> = t
                .iter()
                .filter(|x| f.iter().any(|y| y.to_lowercase() == x.to_lowercase()))
                .collect();
            if !overlap.is_empty() {
                anyhow::bail!("field `{field}`: to_bool true/false sets overlap on {overlap:?}");
            }
            Transform::ToBool { t, f }
        }
        TransformCfg::Split { sep, trim, max } => {
            if sep.is_empty() {
                anyhow::bail!("field `{field}`: split.sep must not be empty");
            }
            Transform::Split {
                sep: sep.clone(),
                trim: *trim,
                max: *max,
            }
        }
        TransformCfg::Join { sep } => Transform::Join { sep: sep.clone() },
        TransformCfg::Unique => Transform::Unique,
        TransformCfg::Sort => Transform::Sort,
        TransformCfg::Take { count } => Transform::Take { count: *count },
        TransformCfg::Drop { count } => Transform::Drop { count: *count },
        TransformCfg::Truncate { max_chars } => Transform::Truncate {
            max_chars: *max_chars,
        },
        TransformCfg::ReplaceEmpty { with } => Transform::ReplaceEmpty { with: with.clone() },
        TransformCfg::Prefix { with } => Transform::Prefix { with: with.clone() },
        TransformCfg::Suffix { with } => Transform::Suffix { with: with.clone() },
    })
}

/// The per-target unit the engine and distributed workers apply to every
/// extracted record: transform chains, then the declared-schema layer.
#[derive(Default)]
pub struct TargetPipeline {
    pub chain: Pipeline,
    pub schema: Option<Schema>,
}

/// What one `apply` call decided about a record.
pub struct PipelineOutcome {
    /// Fields repaired by coercion (each counts one `schema_drifts`).
    pub drift: usize,
    /// `Some(reasons)` only in strict mode: the record was appended to the
    /// quarantine JSONL and must NOT reach the sink. Discovery may still
    /// proceed — a schema violation is a data outcome, not a fetch failure.
    pub quarantined: Option<Vec<String>>,
    /// Fields the pipeline could not repair (transform step errors and, in
    /// coerce mode, unrepairable type violations). These are counted
    /// separately from `drift`: drift is a repaired value, a failure is one
    /// that ships raw or goes to quarantine.
    pub failures: usize,
}

impl TargetPipeline {
    pub fn compile(t: &TargetCfg) -> anyhow::Result<Self> {
        Ok(Self {
            chain: Pipeline::compile(&t.extract)?,
            schema: Schema::compile(t)?,
        })
    }

    /// No transforms and no schema: the legacy pass-through; `apply` then
    /// costs one branch.
    pub fn is_identity(&self) -> bool {
        self.chain.is_identity() && self.schema.is_none()
    }

    pub fn apply(
        &self,
        fields: &mut Map<String, Value>,
        url: &str,
        depth: usize,
    ) -> anyhow::Result<PipelineOutcome> {
        if self.is_identity() {
            return Ok(PipelineOutcome {
                drift: 0,
                quarantined: None,
                failures: 0,
            });
        }
        let mut fails: Vec<(String, String)> = Vec::new();
        let mut drift = 0usize;
        if !self.chain.is_identity() {
            fails = self
                .chain
                .apply(fields)
                .into_iter()
                .map(|i| (i.field, format!("{}: {}", i.step, i.reason)))
                .collect();
        }
        let mut strict = false;
        if let Some(sc) = &self.schema {
            let rep = sc.apply(fields, std::mem::take(&mut fails));
            fails = rep.failures;
            drift += rep.coerced.len();
            strict = sc.enforce == Enforce::Strict;
            if strict && !fails.is_empty() {
                sc.quarantine(url, depth, fields, &fails)?;
            }
        }
        if !fails.is_empty() && !strict {
            // Without strict enforcement every repairable-looking failure is
            // drift; without a schema at all, transform failures are drift
            // too (record written with the raw value).
            drift += fails.len();
        }
        let quarantined = if strict && !fails.is_empty() {
            Some(
                fails
                    .iter()
                    .map(|(f, r)| format!("{f}: {r}"))
                    .collect::<Vec<String>>(),
            )
        } else {
            None
        };
        Ok(PipelineOutcome {
            drift,
            quarantined,
            failures: fails.len(),
        })
    }
}

// ---------------------------------------------------------------- utilities

pub(crate) fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for ch in s.trim().chars() {
        if ch.is_whitespace() || ch == '\u{a0}' {
            in_ws = true;
            continue;
        }
        if in_ws && !out.is_empty() {
            out.push(' ');
        }
        in_ws = false;
        out.push(ch);
    }
    out
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "text",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}

pub(crate) fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut cut: String = s.chars().take(n).collect();
    cut.push('…');
    cut
}

/// Minimal HTML tag stripper with entity decoding. Not a parser: it removes
/// `<...>` spans (respecting quoted attribute brackets) and decodes the five
/// named entities plus numeric refs. Script/style contents become text — the
/// caller's robots/schema story, not ours.
pub(crate) fn dehtml(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            // skip to matching '>', tolerating quoted '>' inside attributes
            let mut quote: Option<u8> = None;
            let mut j = i + 1;
            while j < b.len() {
                let c = b[j];
                if let Some(q) = quote {
                    if c == q {
                        quote = None;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = Some(c);
                } else if c == b'>' {
                    break;
                }
                j += 1;
            }
            i = (j + 1).min(b.len());
            continue;
        }
        if b[i] == b'&' {
            if let Some((decoded, consumed)) = entity(s, i) {
                out.push_str(&decoded);
                i += consumed;
                continue;
            }
        }
        // copy one full char (never a raw byte: multi-byte UTF-8 safe)
        let ch = s[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Decode the entity at `at`; returns (text, bytes consumed).
fn entity(s: &str, at: usize) -> Option<(String, usize)> {
    let rest = &s[at..];
    let end = rest.find(';')?;
    let name = &rest[1..end];
    let total = end + 1;
    let decoded = match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        _ if name.starts_with("#x") || name.starts_with("#X") => {
            let cp = u32::from_str_radix(&name[2..], 16).ok()?;
            return char::from_u32(cp).map(|c| (c.to_string(), total));
        }
        _ if name.starts_with('#') => {
            let cp = name[1..].parse::<u32>().ok()?;
            return char::from_u32(cp).map(|c| (c.to_string(), total));
        }
        _ => return None,
    };
    Some((decoded.to_string(), total))
}

// ---------------------------------------------------------------- numbers

/// Currency/quantity parsing with explicit thousands/decimal disambiguation
/// and accounting-style negatives `(42)`.
pub(crate) fn parse_number(s: &str, as_int: bool) -> Result<Value, String> {
    let t = s.trim();
    let t = t
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .map(|x| format!("-{x}"))
        .unwrap_or_else(|| t.to_string());
    // keep digits . , - + e E; drop currency symbols/spaces/nbsp/percent-ish
    let cleaned: String = t
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | 'e' | 'E'))
        .collect();
    if cleaned.is_empty() || cleaned == "-" || cleaned == "+" {
        return Err(format!("no number in `{}`", trunc(s, 80)));
    }
    // decimal separator: the LAST separator that is followed by exactly 1..2
    // digits at end, else '.'-normalization by position rule:
    let mut normal = String::new();
    let bytes: Vec<char> = cleaned.chars().collect();
    let last_dot = bytes.iter().rposition(|c| *c == '.');
    let last_comma = bytes.iter().rposition(|c| *c == ',');
    let dec = match (last_dot, last_comma) {
        (Some(d), Some(c)) => Some(d.max(c)),
        (Some(d), None) => {
            // single dot: thousands if exactly 3 digits after AND more digits
            // before ("1.005" here means 1005 — a locale-independent rule;
            // pass a JSON *number* when you mean the fraction, see below).
            if bytes.len() - d - 1 == 3 && d > 0 {
                None
            } else {
                Some(d)
            }
        }
        (None, Some(c)) => {
            if bytes.len() - c - 1 == 3 && c > 0 {
                None
            } else {
                Some(c)
            }
        }
        _ => None,
    };
    for (i, c) in bytes.iter().enumerate() {
        match c {
            '.' | ',' => {
                if Some(i) == dec {
                    normal.push('.');
                }
            }
            _ => normal.push(*c),
        }
    }
    let value: f64 = normal
        .parse()
        .map_err(|_| format!("`{s}` is not a number"))?;
    if as_int {
        let r = if value >= 0.0 {
            (value + 0.5).floor()
        } else {
            (value - 0.5).ceil()
        };
        if r.abs() < i64::MAX as f64 {
            return Ok(Value::Number((r as i64).into()));
        }
    }
    Ok(Value::Number(
        Number::from_f64(value).ok_or("non-finite number")?,
    ))
}

fn round_number(n: &Number, decimals: u32) -> Result<Value, String> {
    let Some(f) = n.as_f64() else {
        return Ok(Value::Number(n.clone())); // absurdly large int: already integral
    };
    let p = 10f64.powi(decimals as i32);
    let r = (f * p).round_ties_even() / p;
    if decimals == 0 && r.fract() == 0.0 {
        return Ok(Value::Number((r as i64).into()));
    }
    Ok(Value::Number(Number::from_f64(r).ok_or("non-finite")?))
}

fn round_to(s: &str, decimals: u32) -> Result<Value, String> {
    let v = parse_number(s, false)?;
    let f = v.as_f64().ok_or("not a number")?;
    let p = 10f64.powi(decimals as i32);
    let r = (f * p).round_ties_even() / p;
    if decimals == 0 && r.fract() == 0.0 {
        return Ok(Value::Number((r as i64).into()));
    }
    Ok(Value::Number(Number::from_f64(r).ok_or("non-finite")?))
}

#[derive(Debug, PartialEq)]
pub(crate) struct Quantity {
    pub value: f64,
    pub unit: String,
}

/// Split "1,024 KB" into (1024.0, "kb"). Number uses `parse_number` rules.
pub(crate) fn split_quantity(s: &str) -> Result<Quantity, String> {
    let t = collapse_ws(s).to_lowercase();
    // Numeric part can never contain a letter: split at the FIRST alphabetic
    // char and take the trailing alphanumerics as the unit.
    let split_at = t
        .char_indices()
        .find(|(_, c)| c.is_alphabetic() || *c == '%')
        .map(|(i, _)| i)
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split_at);
    let unit: String = unit
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '%')
        .collect::<String>();
    let v = match parse_number(num.trim(), false) {
        Ok(v) => v.as_f64().ok_or("not numeric")?,
        Err(_) => return Err(format!("`{}` is not a quantity", trunc(s, 60))),
    };
    Ok(Quantity { value: v, unit })
}

pub(crate) fn convert_units(s: &str, to: UnitTarget) -> Result<Value, String> {
    let q = split_quantity(s)?;
    let factor: Option<f64> = match to {
        UnitTarget::Bytes => match q.unit.as_str() {
            // no unit at all: a count of bytes is already normalized
            "" | "b" | "byte" | "bytes" => Some(1.0),
            "kb" => Some(1e3),
            "kib" => Some(1024.0),
            "mb" => Some(1e6),
            "mib" => Some(1024f64.powi(2)),
            "gb" => Some(1e9),
            "gib" => Some(1024f64.powi(3)),
            "tb" => Some(1e12),
            "tib" => Some(1024f64.powi(4)),
            _ => None,
        },
        UnitTarget::Seconds => match q.unit.as_str() {
            "" => Some(1.0),
            "ns" => Some(1e-9),
            "us" | "µs" => Some(1e-6),
            "ms" => Some(1e-3),
            "s" | "sec" | "secs" | "second" | "seconds" => Some(1.0),
            "m" | "min" | "mins" | "minute" | "minutes" => Some(60.0),
            "h" | "hr" | "hrs" | "hour" | "hours" => Some(3600.0),
            "d" | "day" | "days" => Some(86_400.0),
            "w" | "week" | "weeks" => Some(604_800.0),
            _ => None,
        },
        UnitTarget::Percent => match q.unit.as_str() {
            "%" | "pct" | "percent" | "percentage" => Some(0.01),
            "" => None, // a bare number under percent is ambiguous — refuse
            _ => None,
        },
    };
    let Some(f) = factor else {
        return Err(format!("unknown unit `{}` for target {:?}", q.unit, to));
    };
    let v = q.value * f;
    if v.fract() == 0.0 && v.abs() < i64::MAX as f64 {
        return Ok(Value::Number((v as i64).into()));
    }
    Ok(Value::Number(Number::from_f64(v).ok_or("non-finite")?))
}

// ---------------------------------------------------------------- dates

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Days since unix epoch for a proleptic-Gregorian date (Howard Hinnant's
/// civil algorithm; the fetcher's Retry-After parser is the inverse).
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// Normalize a date-ish string to RFC3339 UTC or unix seconds. Supported
/// inputs (in order): epoch seconds/millis, full RFC3339 (any offset),
/// YYYY-MM-DD, IMF-fixdate (RFC 7231), MM/DD/YYYY.
pub(crate) fn parse_date(s: &str, unix_out: bool) -> Result<Value, String> {
    let t = collapse_ws(s);
    if t.is_empty() {
        return Err("empty date".into());
    }
    let secs: i64 = if let Ok(n) = t.parse::<i64>() {
        if n > 10_000_000_000 {
            n / 1000 // millisecond epoch heuristic
        } else {
            n
        }
    } else if let Some(v) = rfc3339_epoch(&t) {
        v
    } else if let Some(v) = imf_epoch(&t) {
        v
    } else if let Some(v) = slash_us_date(&t) {
        v
    } else if let Some(v) = plain_iso_date(&t) {
        v
    } else {
        return Err(format!("unrecognized date `{}`", trunc(&t, 60)));
    };
    if unix_out {
        return Ok(Value::Number(secs.into()));
    }
    Ok(Value::String(epoch_rfc3339(secs)))
}

/// "2015-10-21T07:28:00Z" / with fractional secs / offset "…+02:00".
fn rfc3339_epoch(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once(['T', 't'])?;
    let (y, m, d) = iso_ymd(date)?;
    let rest = rest.trim_end_matches(['Z', 'z']);
    let (clock, off) = match rest.rfind(['+', '-']) {
        Some(idx) => {
            let sign = if rest.as_bytes()[idx] == b'-' { -1 } else { 1 };
            (rest[..idx].to_string(), Some((sign, &rest[idx + 1..])))
        }
        None => (rest.to_string(), None),
    };
    let (hh, mm, ss) = parse_clock(clock.split('.').next()?)?;
    let mut secs = days_from_civil(y, m, d) * 86400 + hh * 3600 + mm * 60 + ss;
    if let Some((sign, o)) = off {
        let (oh, om) = o.split_once(':').or(Some((o, "00")))?;
        secs -= sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60);
    }
    Some(secs)
}

fn parse_clock(c: &str) -> Option<(i64, i64, i64)> {
    let mut it = c.split(':');
    let h = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let s = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((h, m, s))
}

fn iso_ymd(s: &str) -> Option<(i64, u32, u32)> {
    let mut it = s.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !valid_ymd(y, m, d) {
        return None;
    }
    Some((y, m, d))
}

/// Calendar check (leap years included) — "02/30/2015" must not become a date.
fn valid_ymd(y: i64, m: u32, d: u32) -> bool {
    if !(1..=12).contains(&m) || d == 0 {
        return false;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let len: u32 = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    d <= len
}

fn plain_iso_date(s: &str) -> Option<i64> {
    let (y, m, d) = iso_ymd(s)?;
    Some(days_from_civil(y, m, d) * 86400)
}

fn slash_us_date(s: &str) -> Option<i64> {
    let mut it = s.split('/');
    let mo: u32 = it.next()?.parse().ok()?;
    let dy: u32 = it.next()?.parse().ok()?;
    let yr: i64 = it.next()?.parse().ok()?;
    if it.next().is_some() || !valid_ymd(yr, mo, dy) {
        return None;
    }
    Some(days_from_civil(yr, mo, dy) * 86400)
}

/// "Wed, 21 Oct 2015 07:28:00 GMT" (+ CEST-style zones ignored to minutes if
/// named: only GMT/UTC assumed offset 0 — HTTP dates are required to be GMT).
pub(crate) fn imf_epoch(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    let d: u32 = parts[1].parse().ok()?;
    let mon = MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(parts[2]))? as u32
        + 1;
    let y: i64 = parts[3].parse().ok()?;
    if !valid_ymd(y, mon, d) {
        return None;
    }
    let mut it = parts[4].split(':');
    let hh: i64 = it.next()?.parse().ok()?;
    let mm: i64 = it.next()?.parse().ok()?;
    let ss: i64 = it.next()?.parse().ok()?;
    Some(days_from_civil(y, mon, d) * 86400 + hh * 3600 + mm * 60 + ss)
}

pub(crate) fn epoch_rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let (hh, rest) = (rem / 3600, rem % 3600);
    let (mm, ss) = (rest / 60, rest % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

// calendar math lives in `crate::ids` — single authority (see sink notes)
use crate::ids::civil_from_days;

// ---------------------------------------------------------------- PII

const EMAIL: &str = r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}";
const PHONE: &str = r"\+?\d[\d ()\.\-]{6,18}\d";
const CARD: &str = r"\b(?:\d[ \-]?){13,19}\b";
const IPV4: &str = r"\b\d{1,3}(?:\.\d{1,3}){3}\b";

pub(crate) fn mask_pii(
    s: &str,
    email: bool,
    phone: bool,
    card: bool,
    ipv4: bool,
    strategy: MaskStrategy,
) -> String {
    let mut out = s.to_string();
    if email {
        if let Ok(re) = Regex::new(EMAIL) {
            out = re
                .replace_all(&out, |caps: &regex::Captures| {
                    apply_mask(&caps[0], strategy)
                })
                .into_owned();
        }
    }
    if card {
        if let Ok(re) = Regex::new(CARD) {
            out = re
                .replace_all(&out, |caps: &regex::Captures| {
                    let m = &caps[0];
                    // Luhn-validated only: prices like "123456789012345" are
                    // rare in prose, and near-miss digit noise stays intact.
                    if luhn_ok(m) {
                        apply_mask(m, strategy)
                    } else {
                        m.to_string()
                    }
                })
                .into_owned();
        }
    }
    if ipv4 {
        if let Ok(re) = Regex::new(IPV4) {
            out = re
                .replace_all(&out, |caps: &regex::Captures| {
                    let m = &caps[0];
                    if m.split('.').all(|o| o.parse::<u8>().is_ok()) {
                        apply_mask(m, strategy)
                    } else {
                        m.to_string()
                    }
                })
                .into_owned();
        }
    }
    if phone {
        if let Ok(re) = Regex::new(PHONE) {
            out = re
                .replace_all(&out, |caps: &regex::Captures| {
                    let m = &caps[0];
                    let digits = m.chars().filter(|c| c.is_ascii_digit()).count();
                    // 7..=15 digits and not already-handled shapes (card is
                    // longer, ip shorter — digit count disambiguates).
                    if (7..=15).contains(&digits) {
                        apply_mask(m, strategy)
                    } else {
                        m.to_string()
                    }
                })
                .into_owned();
        }
    }
    out
}

pub(crate) fn apply_mask(m: &str, strategy: MaskStrategy) -> String {
    match strategy {
        MaskStrategy::Drop => String::new(),
        MaskStrategy::Hash => {
            let mut h = Sha256::new();
            h.update(m.as_bytes());
            let digest = h.finalize();
            let mut hex = String::with_capacity(8);
            for byte in digest.iter().take(4) {
                hex.push_str(&format!("{byte:02x}"));
            }
            let head: String = m.chars().take(2).collect();
            format!("{head}#{hex}")
        }
        MaskStrategy::Redact => {
            let n = m.chars().count();
            if n <= 4 {
                "*".repeat(n)
            } else {
                let head: String = m.chars().take(2).collect();
                let tail: String = m
                    .chars()
                    .rev()
                    .take(2)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                format!("{head}***{tail}")
            }
        }
    }
}

/// Luhn checksum over digit chars (spaces/dashes ignored).
pub(crate) fn luhn_ok(s: &str) -> bool {
    let digits: Vec<u32> = s
        .chars()
        .filter(|c| c.is_ascii_digit())
        .filter_map(|c| c.to_digit(10))
        .collect();
    if digits.len() < 13 || digits.len() > 19 {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                *d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests;
