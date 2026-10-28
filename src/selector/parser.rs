//! Selector parser and canonicalization for CSS, JSONPath-lite and common
//! locator forms.
use super::ast::{SelectorKind, SelectorNode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "locator", content = "value", rename_all = "snake_case")]
pub enum Locator {
    Css(String),
    Text(String),
    Role {
        name: String,
        accessible_name: Option<String>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedSelector {
    pub kind: SelectorKind,
    pub canonical: String,
    pub attribute: Option<String>,
}

pub fn parse_css(input: &str) -> Result<String, String> {
    let value = normalize_spaces(input);
    if value.is_empty() || value.len() > 4096 {
        return Err("CSS selector is empty or too long".into());
    }
    scraper::Selector::parse(&value).map_err(|e| format!("invalid CSS selector: {e:?}"))?;
    Ok(value)
}
pub fn parse_json_path(input: &str) -> Result<String, String> {
    let value = input.trim();
    if value.is_empty() || value.len() > 4096 {
        return Err("JSONPath is empty or too long".into());
    }
    crate::extract::compile_path(value).map_err(|e| format!("invalid JSONPath: {e}"))?;
    Ok(value.to_owned())
}
pub fn parse_locator(input: &str) -> Result<Locator, String> {
    let v = input.trim();
    if let Some(css) = v.strip_prefix("css=") {
        return parse_css(css).map(Locator::Css);
    }
    if let Some(text) = v.strip_prefix("text=") {
        if text.is_empty() || text.len() > 1024 {
            return Err("text locator is empty or too long".into());
        }
        return Ok(Locator::Text(text.to_owned()));
    }
    if let Some(role) = v.strip_prefix("role=") {
        let (name, accessible_name) = if let Some((n, a)) = role.split_once("[name=") {
            (
                n,
                Some(a.trim_end_matches(']').trim_matches('"').to_owned()),
            )
        } else {
            (role, None)
        };
        if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
            return Err("role locator is invalid".into());
        }
        return Ok(Locator::Role {
            name: name.to_owned(),
            accessible_name,
        });
    }
    parse_css(v).map(Locator::Css)
}
pub fn normalize(
    input: &str,
    kind: SelectorKind,
    attribute: Option<&str>,
) -> Result<ParsedSelector, String> {
    let canonical = if kind == SelectorKind::JsonField {
        parse_json_path(input)?
    } else {
        parse_css(input)?
    };
    if kind == SelectorKind::Attribute && attribute.is_none_or(str::is_empty) {
        return Err("attribute selector requires an attribute name".into());
    }
    Ok(ParsedSelector {
        kind,
        canonical,
        attribute: attribute.map(ToOwned::to_owned),
    })
}
pub fn node_from_css(
    id: &str,
    name: &str,
    selector: &str,
    kind: SelectorKind,
) -> Result<SelectorNode, String> {
    let canonical = parse_css(selector)?;
    Ok(SelectorNode {
        id: id.to_owned(),
        name: name.to_owned(),
        selector: canonical,
        kind,
        ..Default::default()
    })
}
fn normalize_spaces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut whitespace = false;
    for ch in input.trim().chars() {
        if ch.is_whitespace() {
            whitespace = true;
            continue;
        }
        let current_combinator = matches!(ch, '>' | '+' | '~' | ',');
        let previous_combinator = out
            .chars()
            .last()
            .is_some_and(|c| matches!(c, '>' | '+' | '~' | ','));
        if whitespace && !out.is_empty() && !current_combinator && !previous_combinator {
            out.push(' ');
        }
        whitespace = false;
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_css_locator_and_normalizes() {
        assert_eq!(
            parse_locator("css= div.card ").unwrap(),
            Locator::Css("div.card".into())
        );
        assert_eq!(
            parse_locator("text=Load more").unwrap(),
            Locator::Text("Load more".into())
        );
        assert_eq!(parse_css("div   .card").unwrap(), "div .card");
    }
    #[test]
    fn rejects_invalid_css() {
        assert!(parse_css("div[").is_err());
    }
}
