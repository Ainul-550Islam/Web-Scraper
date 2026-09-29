use super::*;

use crate::config::FieldCfg;
use std::collections::BTreeMap;

fn bare() -> FieldCfg {
    FieldCfg {
        css: ".x".into(),
        path: None,
        attr: None,
        all: false,
        transforms: Vec::new(),
    }
}

fn target(schema: SchemaCfg) -> TargetCfg {
    let mut t = TargetCfg {
        id: "t".into(),
        schema,
        ..Default::default()
    };
    t.extract.insert(
        "price".into(),
        FieldCfg {
            css: ".p".into(),
            path: None,
            attr: None,
            all: false,
            transforms: vec![crate::config::TransformCfg::ParseNumber { as_int: false }],
        },
    );
    t
}

fn fields(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn off_and_no_fields_compiles_to_none() {
    let s = Schema::compile(&target(SchemaCfg::default())).unwrap();
    assert!(
        s.is_none(),
        "legacy config must have no schema object at all"
    );
}

#[test]
fn strict_without_quarantine_path_is_a_config_error() {
    let cfg = SchemaCfg {
        enforce: "strict".into(),
        fields: BTreeMap::from([("price".to_string(), "number".to_string())]),
        quarantine_path: None,
    };
    let err = Schema::compile(&target(cfg)).unwrap_err().to_string();
    assert!(err.contains("quarantine"), "{err}");
}

#[test]
fn coerce_repairs_text_number_and_counts_it() {
    let cfg = SchemaCfg {
        enforce: "coerce".into(),
        fields: BTreeMap::from([("price".to_string(), "number".to_string())]),
        quarantine_path: None,
    };
    let s = Schema::compile(&target(cfg)).unwrap().unwrap();
    let mut f = fields(&[("price", Value::String("42.5".into()))]);
    let rep = s.apply(&mut f, Vec::new());
    assert_eq!(rep.coerced, vec!["price".to_string()]);
    assert_eq!(f["price"], Value::from(42.5));
    assert!(rep.failures.is_empty());
}

#[test]
fn number_in_int_field_only_recoerces_when_fractional() {
    let cfg = SchemaCfg {
        enforce: "coerce".into(),
        fields: BTreeMap::from([("n".to_string(), "int".to_string())]),
        quarantine_path: None,
    };
    let mut f = fields(&[("n", Value::from(7)), ("x", Value::from(1))]);
    let rep = s_apply(&cfg, &mut f);
    assert!(rep.coerced.is_empty(), "7 is already int-clean: {rep:?}");
    let mut g = fields(&[("n", Value::from(7.4))]);
    let rep = s_apply(&cfg, &mut g);
    assert_eq!(rep.coerced, vec!["n".to_string()]);
    assert_eq!(g["n"], Value::from(7));
}

fn s_apply(cfg: &SchemaCfg, f: &mut Map<String, Value>) -> ApplyReport {
    let mut t = target(cfg.clone());
    t.extract.insert("n".into(), bare());
    let s = Schema::compile(&t).unwrap().unwrap();
    s.apply(f, Vec::new())
}

#[test]
fn text_field_stringifies_scalars_but_not_objects() {
    let cfg = SchemaCfg {
        enforce: "coerce".into(),
        fields: BTreeMap::from([("s".to_string(), "text".to_string())]),
        quarantine_path: None,
    };
    let mut t = target(cfg.clone());
    t.extract.insert("s".into(), bare());
    let s = Schema::compile(&t).unwrap().unwrap();
    let mut f = fields(&[("s", Value::Bool(true))]);
    let rep = s.apply(&mut f, Vec::new());
    assert_eq!(f["s"], Value::String("true".into()));
    assert_eq!(rep.coerced.len(), 1);
    let mut g = fields(&[("s", Value::Object(Map::new()))]);
    let rep = s.apply(&mut g, Vec::new());
    assert!(
        !rep.failures.is_empty(),
        "objects are not silently flattened"
    );
}

#[test]
fn list_coercion_splits_comma_text_and_wraps_scalars() {
    let cfg = SchemaCfg {
        enforce: "coerce".into(),
        fields: BTreeMap::from([("l".to_string(), "list".to_string())]),
        quarantine_path: None,
    };
    let mut t = target(cfg.clone());
    t.extract.insert("l".into(), bare());
    let s = Schema::compile(&t).unwrap().unwrap();
    let mut f = fields(&[("l", Value::String("a, b ,c".into()))]);
    s.apply(&mut f, Vec::new());
    assert_eq!(
        f["l"],
        Value::Array(vec!["a".into(), "b".into(), "c".into()])
    );
    let mut g = fields(&[("l", Value::from(5))]);
    s.apply(&mut g, Vec::new());
    assert_eq!(g["l"], Value::Array(vec![Value::from(5)]));
}

#[test]
fn transform_failures_are_schema_failures_under_strict() {
    let dir = std::env::temp_dir().join(format!("harvest-quadis-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let q = dir.join("quarantine.jsonl");
    let _ = std::fs::remove_file(&q);
    let cfg = SchemaCfg {
        enforce: "strict".into(),
        fields: BTreeMap::new(),
        quarantine_path: Some(q.clone()),
    };
    let s = Schema::compile(&target(cfg)).unwrap().unwrap();
    let issues = vec![(
        "price".to_string(),
        "parse_number: no number in `free!`".to_string(),
    )];
    let f = fields(&[("price", Value::String("free!".into()))]);
    let mut f2 = f.clone();
    let rep = s.apply(&mut f2, issues);
    assert_eq!(rep.failures.len(), 1);
    s.quarantine("http://x/p", 1, &f, &rep.failures).unwrap();
    let line = std::fs::read_to_string(&q).unwrap();
    let v: Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(v["url"], "http://x/p");
    assert_eq!(v["reasons"][0], "price: parse_number: no number in `free!`");
    assert_eq!(v["fields"]["price"], "free!");
    assert!(v["ts"].is_string());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn date_field_normalizes_rfc3339_offsets() {
    let cfg = SchemaCfg {
        enforce: "coerce".into(),
        fields: BTreeMap::from([("d".to_string(), "date".to_string())]),
        quarantine_path: None,
    };
    let mut t = target(cfg.clone());
    t.extract.insert("d".into(), bare());
    let s = Schema::compile(&t).unwrap().unwrap();
    let mut f = fields(&[("d", Value::String("2015-10-21T09:28:00+02:00".into()))]);
    let rep = s.apply(&mut f, Vec::new());
    assert_eq!(f["d"], Value::String("2015-10-21T07:28:00Z".into()));
    assert_eq!(rep.coerced, vec!["d".to_string()]);
}

#[test]
fn absent_fields_are_not_violations() {
    let cfg = SchemaCfg {
        enforce: "strict".into(),
        fields: BTreeMap::from([("missing".to_string(), "number".to_string())]),
        quarantine_path: Some("/tmp/never-used.jsonl".into()),
    };
    let mut t = target(cfg.clone());
    t.extract.insert("missing".into(), bare());
    let s = Schema::compile(&t).unwrap().unwrap();
    let mut f = Map::new();
    let rep = s.apply(&mut f, Vec::new());
    assert!(
        rep.failures.is_empty(),
        "absence is the extractor's contract"
    );
}

#[test]
fn json_schema_export_pins_types_and_required() {
    let cfg = SchemaCfg {
        enforce: "coerce".into(),
        fields: BTreeMap::from([("price".to_string(), "number".to_string())]),
        quarantine_path: None,
    };
    let t = target(cfg);
    let v = json_schema_for(&t);
    assert_eq!(v["$schema"], "http://json-schema.org/draft-07/schema#");
    assert_eq!(v["title"], "harvest:t");
    assert_eq!(v["properties"]["price"]["type"], "number");
    assert_eq!(v["required"][0], "price");
    assert_eq!(v["additionalProperties"], Value::Bool(false));
    // transform chain shows up as x-metadata
    let chain = v["properties"]["price"]["harvest:transforms"].clone();
    assert_eq!(chain[0], "parse_number(as_int=false)");
}

#[test]
fn all_matches_infer_array_type() {
    let mut t = TargetCfg::default();
    t.extract.insert(
        "tags".into(),
        FieldCfg {
            css: ".tag".into(),
            path: None,
            attr: None,
            all: true,
            transforms: vec![crate::config::TransformCfg::Lowercase],
        },
    );
    let v = json_schema_for(&t);
    assert_eq!(v["properties"]["tags"]["type"], "array");
}
