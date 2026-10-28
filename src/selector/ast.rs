//! Typed hierarchical selector AST for flat and nested/repeated records.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectorKind {
    #[default]
    Text,
    Element,
    Attribute,
    Link,
    Image,
    Html,
    JsonField,
    RepeatedRecord,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transform", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transform {
    Trim,
    NormalizeWhitespace,
    Lowercase,
    Uppercase,
    ParseNumber,
    ParseInt,
    Prefix {
        value: String,
    },
    Suffix {
        value: String,
    },
    RegexReplace {
        pattern: String,
        replacement: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SelectorNode {
    pub id: String,
    pub name: String,
    pub kind: SelectorKind,
    pub selector: String,
    pub attribute: Option<String>,
    pub required: bool,
    pub repeat: bool,
    pub transforms: Vec<Transform>,
    pub children: Vec<SelectorNode>,
}
impl Default for SelectorNode {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: SelectorKind::Text,
            selector: String::new(),
            attribute: None,
            required: false,
            repeat: false,
            transforms: Vec::new(),
            children: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SelectorTree {
    pub version: u32,
    pub root: Vec<SelectorNode>,
}
impl SelectorTree {
    pub fn validate(&self) -> Result<()> {
        if self.version > 1 {
            bail!("selector tree version is newer than this runtime");
        }
        if self.root.is_empty() {
            bail!("selector tree must contain at least one root node");
        }
        let mut ids = BTreeSet::new();
        let mut count = 0;
        for n in &self.root {
            validate_node(n, 0, &mut ids, &mut count)?;
        }
        if count > 512 {
            bail!("selector tree supports at most 512 nodes");
        }
        Ok(())
    }
}
fn validate_node(
    node: &SelectorNode,
    depth: usize,
    ids: &mut BTreeSet<String>,
    count: &mut usize,
) -> Result<()> {
    *count += 1;
    if depth > 8 {
        bail!("selector tree nesting exceeds 8 levels");
    }
    if node.id.is_empty()
        || node.id.len() > 128
        || !node
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        bail!("selector node id has an invalid shape");
    }
    if !ids.insert(node.id.clone()) {
        bail!("duplicate selector node id `{}`", node.id);
    }
    if node.name.is_empty() || node.name.len() > 128 || node.name.chars().any(char::is_control) {
        bail!("selector node name is invalid");
    }
    if node.selector.is_empty()
        || node.selector.len() > 4096
        || node.selector.chars().any(char::is_control)
    {
        bail!("selector node `{}` has an invalid selector", node.name);
    }
    if node.kind == SelectorKind::Attribute && node.attribute.as_deref().is_none_or(str::is_empty) {
        bail!(
            "attribute selector `{}` requires an attribute name",
            node.name
        );
    }
    if node.kind == SelectorKind::RepeatedRecord && node.children.is_empty() {
        bail!("repeated record `{}` must have child fields", node.name);
    }
    if node.children.len() > 128 || node.transforms.len() > 32 {
        bail!(
            "selector node `{}` has too many children/transforms",
            node.name
        );
    }
    for transform in &node.transforms {
        if let Transform::RegexReplace { pattern, .. } = transform {
            regex::Regex::new(pattern).map_err(|e| {
                anyhow::anyhow!(
                    "selector node `{}` has invalid transform regex: {e}",
                    node.name
                )
            })?;
        }
    }
    let mut names = BTreeSet::new();
    for child in &node.children {
        if !names.insert(child.name.clone()) {
            bail!(
                "selector node `{}` has duplicate child field `{}`",
                node.name,
                child.name
            );
        }
        validate_node(child, depth + 1, ids, count)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(id: &str, name: &str) -> SelectorNode {
        SelectorNode {
            id: id.into(),
            name: name.into(),
            selector: ".x".into(),
            ..Default::default()
        }
    }
    #[test]
    fn rejects_duplicate_ids() {
        let t = SelectorTree {
            version: 1,
            root: vec![node("x", "a"), node("x", "b")],
        };
        assert!(t.validate().is_err());
    }
    #[test]
    fn rejects_repeat_without_children() {
        let mut n = node("x", "items");
        n.kind = SelectorKind::RepeatedRecord;
        let t = SelectorTree {
            version: 1,
            root: vec![n],
        };
        assert!(t.validate().is_err());
    }
}
