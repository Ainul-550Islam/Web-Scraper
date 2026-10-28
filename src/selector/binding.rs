//! Selector-tree output binding, transformation and schema validation.
use super::ast::{SelectorTree, Transform};
use super::record::Record;
use crate::browser::error::{BrowserError, BrowserResult};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn apply_transforms(mut value: Value, transforms: &[Transform]) -> BrowserResult<Value> {
    for transform in transforms {
        if value.is_null() {
            break;
        }
        let text = value
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| value.to_string());
        value = match transform {
            Transform::Trim => Value::String(text.trim().to_owned()),
            Transform::NormalizeWhitespace => {
                Value::String(text.split_whitespace().collect::<Vec<_>>().join(" "))
            }
            Transform::Lowercase => Value::String(text.to_lowercase()),
            Transform::Uppercase => Value::String(text.to_uppercase()),
            Transform::Prefix { value: prefix } => Value::String(format!("{prefix}{text}")),
            Transform::Suffix { value: suffix } => Value::String(format!("{text}{suffix}")),
            Transform::ParseNumber => {
                let n = text
                    .trim()
                    .replace(',', "")
                    .parse::<f64>()
                    .map_err(|_| BrowserError::InvalidInput("number transform"))?;
                serde_json::Number::from_f64(n)
                    .map(Value::Number)
                    .ok_or(BrowserError::InvalidInput("non-finite number transform"))?
            }
            Transform::ParseInt => Value::Number(
                text.trim()
                    .replace(',', "")
                    .parse::<i64>()
                    .map_err(|_| BrowserError::InvalidInput("integer transform"))?
                    .into(),
            ),
            Transform::RegexReplace {
                pattern,
                replacement,
            } => {
                let re = regex::Regex::new(pattern)
                    .map_err(|_| BrowserError::InvalidInput("transform regex"))?;
                Value::String(re.replace_all(&text, replacement.as_str()).into_owned())
            }
        };
    }
    Ok(value)
}

pub fn bind(
    records: Vec<Record>,
    tree: &SelectorTree,
    schema: &BTreeMap<String, String>,
    strict: bool,
) -> BrowserResult<Vec<Value>> {
    tree.validate()
        .map_err(|e| BrowserError::Config(e.to_string()))?;
    let mut out = Vec::with_capacity(records.len());
    for record in records {
        let mut value = record.to_value();
        if let Some(object) = value.as_object_mut() {
            for (field, kind) in schema {
                if let Some(v) = object.get(field) {
                    if !schema_type_matches(kind, v) {
                        if strict {
                            return Err(BrowserError::InvalidInput("selector output schema"));
                        }
                        let coerced = coerce(kind, v.clone());
                        object.insert(field.clone(), coerced);
                    }
                } else if strict {
                    return Err(BrowserError::NotFound("required schema field"));
                }
            }
        }
        out.push(value);
    }
    Ok(out)
}
fn schema_type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "text" => value.is_string() || value.is_null(),
        "number" => value.is_number() || value.is_null(),
        "int" => value.as_i64().is_some() || value.is_null(),
        "bool" => value.is_boolean() || value.is_null(),
        "list" => value.is_array() || value.is_null(),
        "json" => true,
        _ => false,
    }
}
fn coerce(kind: &str, value: Value) -> Value {
    let text = value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string());
    match kind {
        "text" => Value::String(text),
        "number" => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "int" => text
            .parse::<i64>()
            .ok()
            .map(Into::into)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "bool" => match text.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Value::Bool(true),
            "false" | "0" | "no" => Value::Bool(false),
            _ => Value::Null,
        },
        "list" => Value::Array(vec![value]),
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transform_normalizes_and_parses() {
        let value = apply_transforms(
            Value::String(" $1,200.50 ".into()),
            &[Transform::Trim, Transform::ParseNumber],
        )
        .unwrap();
        assert_eq!(value.as_f64(), Some(1200.5));
    }
    #[test]
    fn regex_transforms_are_compiled_before_use() {
        assert!(apply_transforms(
            Value::String("x".into()),
            &[Transform::RegexReplace {
                pattern: "[".into(),
                replacement: "".into()
            }]
        )
        .is_err());
    }
}
