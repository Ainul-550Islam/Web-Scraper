//! Execute a validated selector tree against a bounded HTML snapshot.
use super::ast::{SelectorKind, SelectorNode, SelectorTree};
use super::record::Record;
use crate::browser::error::{BrowserError, BrowserResult};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn extract(html: &str, tree: &SelectorTree, max_records: usize) -> BrowserResult<Vec<Record>> {
    if html.len() > 64 * 1024 * 1024 {
        return Err(BrowserError::ContentLimit {
            kind: "selector HTML input",
            limit: 64 * 1024 * 1024,
        });
    }
    tree.validate()
        .map_err(|e| BrowserError::Config(e.to_string()))?;
    if max_records == 0 || max_records > 100_000 {
        return Err(BrowserError::InvalidInput("selector record limit"));
    }
    let document = Html::parse_document(html);
    let mut records = vec![Record::new("root")];
    let mut counter = 0usize;
    for node in &tree.root {
        apply_node_to_record(
            None,
            &document,
            node,
            &mut records[0],
            &mut counter,
            max_records,
        )?;
    }
    if records[0].children.is_empty() && records[0].fields.is_empty() {
        return Ok(Vec::new());
    }
    Ok(records)
}

/// Execute JSONField nodes against a JSON response. HTML selectors and JSONPath
/// are deliberately kept separate so a JSONPath can never be parsed as CSS.
pub fn extract_json(
    body: &str,
    tree: &SelectorTree,
    max_records: usize,
) -> BrowserResult<Vec<Record>> {
    if body.len() > 64 * 1024 * 1024 {
        return Err(BrowserError::ContentLimit {
            kind: "selector JSON input",
            limit: 64 * 1024 * 1024,
        });
    }
    let json: Value = serde_json::from_str(body)
        .map_err(|_| BrowserError::InvalidInput("JSON selector document"))?;
    extract_json_value(&json, tree, max_records)
}

pub fn extract_json_value(
    json: &Value,
    tree: &SelectorTree,
    max_records: usize,
) -> BrowserResult<Vec<Record>> {
    tree.validate()
        .map_err(|e| BrowserError::Config(e.to_string()))?;
    if max_records == 0 || max_records > 100_000 {
        return Err(BrowserError::InvalidInput("selector record limit"));
    }
    let mut root = Record::new("root");
    let mut counter = 0usize;
    for node in &tree.root {
        apply_json_node(json, node, &mut root, &mut counter, max_records)?;
    }
    if root.children.is_empty() && root.fields.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![root])
}

fn apply_json_node(
    scope: &Value,
    node: &SelectorNode,
    record: &mut Record,
    counter: &mut usize,
    max_records: usize,
) -> BrowserResult<()> {
    if node.kind != SelectorKind::JsonField && node.kind != SelectorKind::RepeatedRecord {
        return Err(BrowserError::InvalidInput(
            "HTML selector used against a JSON document",
        ));
    }
    let path = crate::extract::compile_path(&node.selector)
        .map_err(|_| BrowserError::InvalidInput("JSONPath"))?;
    let selected = crate::extract::query_path(scope, &path);
    if node.kind == SelectorKind::RepeatedRecord || node.repeat {
        if selected.len().saturating_add(*counter) > max_records {
            return Err(BrowserError::ContentLimit {
                kind: "selector records",
                limit: max_records,
            });
        }
        let mut repeated = Vec::with_capacity(selected.len());
        for value in selected {
            *counter = counter.saturating_add(1);
            let mut item = Record::new(format!("{}-{counter}", node.id));
            for child in &node.children {
                apply_json_node(value, child, &mut item, counter, max_records)?;
            }
            repeated.push(item);
        }
        record.children.insert(node.name.clone(), repeated);
        return Ok(());
    }
    if selected.is_empty() && node.required {
        return Err(BrowserError::NotFound("required JSONPath result"));
    }
    let value = match selected.len() {
        0 => Value::Null,
        1 => selected[0].clone(),
        _ => Value::Array(selected.iter().map(|v| (*v).clone()).collect()),
    };
    let transformed = crate::selector::binding::apply_transforms(value.clone(), &node.transforms)?;
    record.fields.insert(node.name.clone(), transformed);
    if !node.children.is_empty() {
        let child_scope = selected.first().copied().unwrap_or(&value);
        for child in &node.children {
            apply_json_node(child_scope, child, record, counter, max_records)?;
        }
    }
    Ok(())
}

fn apply_node_to_record(
    context: Option<ElementRef<'_>>,
    document: &Html,
    node: &SelectorNode,
    record: &mut Record,
    counter: &mut usize,
    max_records: usize,
) -> BrowserResult<()> {
    if node.kind == SelectorKind::JsonField {
        return Err(BrowserError::InvalidInput(
            "JSONPath node used against an HTML document",
        ));
    }
    let selector =
        Selector::parse(&node.selector).map_err(|_| BrowserError::InvalidInput("CSS selector"))?;
    let elements: Vec<ElementRef<'_>> = match context {
        Some(ctx) => ctx.select(&selector).collect(),
        None => document.select(&selector).collect(),
    };
    if node.kind == SelectorKind::RepeatedRecord || node.repeat {
        let mut repeated = Vec::new();
        for element in elements {
            if repeated.len() >= max_records {
                return Err(BrowserError::ContentLimit {
                    kind: "selector records",
                    limit: max_records,
                });
            }
            *counter += 1;
            let mut item = Record::new(format!("{}-{counter}", node.id));
            for child in &node.children {
                apply_node_to_record(
                    Some(element),
                    document,
                    child,
                    &mut item,
                    counter,
                    max_records,
                )?;
            }
            repeated.push(item);
        }
        record.children.insert(node.name.clone(), repeated);
        return Ok(());
    }
    let first = elements.first().copied();
    if first.is_none() {
        if node.required {
            return Err(BrowserError::NotFound("required selector result"));
        }
        record.fields.insert(node.name.clone(), Value::Null);
        return Ok(());
    }
    let value = extract_value(first.unwrap(), node)?;
    record.fields.insert(node.name.clone(), value);
    for child in &node.children {
        apply_node_to_record(first, document, child, record, counter, max_records)?;
    }
    Ok(())
}

fn extract_value(element: ElementRef<'_>, node: &SelectorNode) -> BrowserResult<Value> {
    let value = match node.kind {
        SelectorKind::Element => Value::String(element.value().name().to_owned()),
        SelectorKind::Text => Value::String(element.text().collect::<String>()),
        SelectorKind::Attribute => Value::String(
            element
                .value()
                .attr(node.attribute.as_deref().unwrap_or(""))
                .unwrap_or("")
                .to_owned(),
        ),
        SelectorKind::Link => Value::String(element.value().attr("href").unwrap_or("").to_owned()),
        SelectorKind::Image => Value::String(
            element
                .value()
                .attr("src")
                .or_else(|| element.value().attr("data-src"))
                .unwrap_or("")
                .to_owned(),
        ),
        SelectorKind::Html => Value::String(element.inner_html()),
        SelectorKind::JsonField => {
            let txt = element.text().collect::<String>();
            let json: Value = serde_json::from_str(&txt)
                .map_err(|_| BrowserError::InvalidInput("embedded JSON"))?;
            let path = crate::extract::compile_path(&node.selector)
                .map_err(|_| BrowserError::InvalidInput("JSONPath"))?;
            let selected = crate::extract::query_path(&json, &path);
            if selected.len() == 1 {
                (*selected[0]).clone()
            } else {
                Value::Array(selected.into_iter().cloned().collect())
            }
        }
        SelectorKind::RepeatedRecord => Value::Null,
    };
    crate::selector::binding::apply_transforms(value, &node.transforms)
}

/// Convenience output for a flat consumer; nested records remain arrays of
/// objects rather than being flattened into ambiguous key names.
pub fn extract_values(
    html: &str,
    tree: &SelectorTree,
    max_records: usize,
) -> BrowserResult<Vec<BTreeMap<String, Value>>> {
    Ok(extract(html, tree, max_records)?
        .into_iter()
        .map(|r| {
            let value = r.to_value();
            value
                .as_object()
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::ast::*;
    fn node(id: &str, name: &str, selector: &str, kind: SelectorKind) -> SelectorNode {
        SelectorNode {
            id: id.into(),
            name: name.into(),
            selector: selector.into(),
            kind,
            ..Default::default()
        }
    }
    #[test]
    fn extracts_repeating_nested_records() {
        let mut repeated = node("items", "items", "article", SelectorKind::RepeatedRecord);
        repeated.children = vec![
            node("title", "title", "h2", SelectorKind::Text),
            node("price", "price", ".price", SelectorKind::Text),
        ];
        let tree = SelectorTree {
            version: 1,
            root: vec![repeated],
        };
        let rows=extract("<article><h2>A</h2><b class='price'>$1</b></article><article><h2>B</h2><b class='price'>$2</b></article>",&tree,10).unwrap();
        assert_eq!(rows[0].children["items"].len(), 2);
        assert_eq!(rows[0].children["items"][1].fields["title"], "B");
    }
}
