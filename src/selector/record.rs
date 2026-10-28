//! Record identity and hierarchical field grouping for selector output.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub fields: BTreeMap<String, Value>,
    pub children: BTreeMap<String, Vec<Record>>,
}
impl Record {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            fields: BTreeMap::new(),
            children: BTreeMap::new(),
        }
    }
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        self.fields.insert(key.into(), value);
    }
    pub fn add_child(&mut self, relation: impl Into<String>, record: Record) {
        self.children
            .entry(relation.into())
            .or_default()
            .push(record);
    }
    pub fn field(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }
    pub fn to_value(&self) -> Value {
        let mut map = serde_json::Map::new();
        for (k, v) in &self.fields {
            map.insert(k.clone(), v.clone());
        }
        for (k, v) in &self.children {
            map.insert(
                k.clone(),
                Value::Array(v.iter().map(Record::to_value).collect()),
            );
        }
        Value::Object(map)
    }
    pub fn flatten(&self, prefix: &str, out: &mut BTreeMap<String, Value>) {
        for (k, v) in &self.fields {
            let name = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            out.insert(name, v.clone());
        }
        for (k, children) in &self.children {
            for (i, c) in children.iter().enumerate() {
                c.flatten(
                    &format!(
                        "{prefix}{k}[{i}]{}",
                        if prefix.is_empty() { "" } else { "." }
                    ),
                    out,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_records_preserve_grouping() {
        let mut root = Record::new("root");
        let mut child = Record::new("item-1");
        child.set("title", Value::String("A".into()));
        root.add_child("items", child);
        assert_eq!(root.to_value()["items"][0]["title"], "A");
    }
}
