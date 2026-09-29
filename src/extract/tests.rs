use super::*;

fn rules(pairs: &[(&str, &str)]) -> BTreeMap<String, FieldCfg> {
    pairs
        .iter()
        .map(|(n, css)| {
            (
                n.to_string(),
                FieldCfg {
                    css: css.to_string(),
                    attr: None,
                    all: false,
                    transforms: Vec::new(),
                    path: None,
                },
            )
        })
        .collect()
}

#[test]
fn extracts_text_and_attr() {
    let html = r#"<html><body><h1> Widget&nbsp; 3000 </h1>
        <p class="price">$19.99</p><a class="next" href="/p/2">next</a></body></html>"#;
    let doc = Html::parse_document(html);
    let mut r = rules(&[("title", "h1"), ("price", "p.price")]);
    r.insert(
        "next".into(),
        FieldCfg {
            css: "a.next".into(),
            attr: Some("href".into()),
            all: false,
            transforms: Vec::new(),
            path: None,
        },
    );
    let ex = Extractor::compile(&r).unwrap();
    let m = ex.extract(&doc);
    assert_eq!(m["title"], Value::String("Widget 3000".into()));
    assert_eq!(m["price"], Value::String("$19.99".into()));
    assert_eq!(m["next"], Value::String("/p/2".into()));
}

#[test]
fn missing_field_is_null_and_all_collects() {
    let html = "<html><body><li>a</li><li>b</li></body></html>";
    let doc = Html::parse_document(html);
    let mut r = rules(&[("none", "h2")]);
    r.insert(
        "items".into(),
        FieldCfg {
            css: "li".into(),
            attr: None,
            all: true,
            transforms: Vec::new(),
            path: None,
        },
    );
    let ex = Extractor::compile(&r).unwrap();
    let m = ex.extract(&doc);
    assert_eq!(m["none"], Value::Null);
    assert_eq!(m["items"], Value::Array(vec!["a".into(), "b".into()]));
}

#[test]
fn jsonpath_grammar_is_tight_and_prevalidated() {
    assert_eq!(
        compile_path("$.items[2].name").unwrap(),
        vec![
            PathStep::Key("items".into()),
            PathStep::Index(2),
            PathStep::Key("name".into())
        ]
    );
    assert_eq!(
        compile_path("$.a[*]").unwrap(),
        vec![PathStep::Key("a".into()), PathStep::Wildcard]
    );
    // hyphens/underscores allowed in keys (API payloads love them)
    assert_eq!(
        compile_path("$.x-1_a").unwrap(),
        vec![PathStep::Key("x-1_a".into())]
    );
    for bad in ["a.b", "$", "$.a b", "$.a[", "$.a[x]", "$.a[1e2]", "$..a"] {
        assert!(compile_path(bad).is_err(), "`{bad}` must be rejected");
    }
    // Config::validate surfaces this per-field, before any request:
    let mut r = BTreeMap::new();
    r.insert(
        "n".to_string(),
        FieldCfg {
            css: String::new(),
            path: Some("nope".into()),
            attr: None,
            all: false,
            transforms: Vec::new(),
        },
    );
    let err = JsonExtractor::compile(&r).unwrap_err().to_string();
    assert!(err.contains("field `n`") || err.contains("nope"), "{err}");
}

#[test]
fn query_path_semantics_missing_is_empty_not_panic() {
    let doc = serde_json::json!({
        "items": [ {"name": "a"}, {"name": "b"} ],
        "meta": { "k": 7 }
    });
    let q = |p: &str| query_path(&doc, &compile_path(p).unwrap());
    assert_eq!(q("$.items[0].name").len(), 1);
    assert_eq!(*q("$.items[0].name")[0], serde_json::json!("a"));
    assert!(q("$.items[9].name").is_empty(), "index past end");
    assert!(q("$.missing.deep").is_empty(), "missing key short-circuits");
    assert!(
        q("$.items.name").is_empty(),
        "key on array is no match, not an error"
    );
    assert_eq!(q("$.items[*].name").len(), 2, "wildcard expands in order");
    assert_eq!(*q("$.items[*].name")[1], serde_json::json!("b"));
    assert_eq!(q("$.meta[*]").len(), 1, "object wildcard iterates values");
    assert_eq!(
        q("$[*]").len(),
        2,
        "root wildcard iterates top-level values"
    );
}

#[test]
fn json_extractor_preserves_types_and_first_match() {
    let mut r = BTreeMap::new();
    let mk = |path: &str, all: bool| FieldCfg {
        css: String::new(),
        path: Some(path.into()),
        attr: None,
        all,
        transforms: Vec::new(),
    };
    r.insert("price".into(), mk("$.items[0].price_cents", false));
    r.insert("tags".into(), mk("$.items[*].tags[*]", true));
    r.insert("absent".into(), mk("$.nope", false));
    let ex = JsonExtractor::compile(&r).unwrap();
    let doc = serde_json::json!({ "items": [
        { "price_cents": 1499, "tags": ["widget", "sale"] },
        { "price_cents": 5, "tags": [] }
    ]});
    let m = ex.extract(&doc);
    assert_eq!(m["price"], serde_json::json!(1499), "numbers stay numbers");
    assert_eq!(
        m["tags"],
        serde_json::json!(["widget", "sale" /*second item adds none*/,])
    );
    assert_eq!(
        m["absent"],
        serde_json::Value::Null,
        "missing => null placeholder"
    );
}

#[test]
fn fingerprint_stable_across_whitespace_and_url() {
    let a = fingerprint("<body><p>hello   world</p></body>");
    let b = fingerprint("<body>  <p> hello\n world </p>  </body>");
    assert_eq!(a, b);
    let c = fingerprint("<body><p>different</p></body>");
    assert_ne!(a, c);
    assert_eq!(a.len(), 64);
}
