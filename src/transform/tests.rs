use super::*;

use crate::config::TransformCfg as C;
use serde_json::json;

fn one(cfg: C, input: Value) -> Result<Value, String> {
    let mut rules = BTreeMap::new();
    rules.insert(
        "f".to_string(),
        FieldCfg {
            css: "x".into(),
            path: None,
            attr: None,
            all: false,
            transforms: vec![cfg],
        },
    );
    let p = Pipeline::compile(&rules).unwrap();
    let mut fields = Map::new();
    fields.insert("f".into(), input);
    let issues = p.apply(&mut fields);
    if let Some(i) = issues.first() {
        return Err(format!("{}: {}", i.step, i.reason));
    }
    Ok(fields["f"].clone())
}

fn ok(cfg: C, input: Value) -> Value {
    one(cfg, input).expect("transform should succeed")
}

#[test]
fn text_steps_are_elementwise_over_arrays() {
    let got = ok(C::Trim, json!(["  a ", "b  ", " c"]));
    assert_eq!(got, json!(["a", "b", "c"]));
    // numbers stringify through text steps (documented no-op, not error)
    assert_eq!(ok(C::Uppercase, json!("x")), json!("X"));
    assert_eq!(ok(C::Lowercase, json!("ÄB")), json!("äb"));
}

#[test]
fn collapse_whitespace_treats_nbsp_as_space() {
    assert_eq!(
        ok(C::CollapseWhitespace, json!(" a  b \n c")),
        json!("a b c")
    );
}

#[test]
fn regex_replace_and_extract() {
    assert_eq!(
        ok(
            C::RegexReplace {
                pattern: "o+".into(),
                replacement: "0".into()
            },
            json!("fooo bar")
        ),
        json!("f0 bar")
    );
    assert_eq!(
        ok(
            C::RegexExtract {
                pattern: r"v(\d+)".into(),
                group: 1
            },
            json!("harvest v2 released")
        ),
        json!("2")
    );
    let err = one(
        C::RegexExtract {
            pattern: r"v(\d+)".into(),
            group: 1,
        },
        json!("no version"),
    )
    .unwrap_err();
    assert!(err.contains("no match"), "{err}");
    let err = one(
        C::RegexExtract {
            pattern: "(x)(y)".into(),
            group: 5,
        },
        json!("xy"),
    )
    .unwrap_err();
    assert!(err.contains("group 5"), "{err}");
}

#[test]
fn bad_regex_is_a_compile_error_naming_the_field() {
    let mut rules = BTreeMap::new();
    rules.insert(
        "f".to_string(),
        FieldCfg {
            css: "x".into(),
            path: None,
            attr: None,
            all: false,
            transforms: vec![C::RegexReplace {
                pattern: "([unclosed".into(),
                replacement: String::new(),
            }],
        },
    );
    let err = Pipeline::compile(&rules).unwrap_err().to_string();
    assert!(
        err.contains("field `f`") && err.contains("regex_replace"),
        "{err}"
    );
}

#[test]
fn strip_tags_decodes_entities_utf8_safe() {
    let src = "<b>caf\u{e9} &amp; th\u{e9}</b><i class=\"a>b\">x</i>&#65;&#x42;&nbsp;&unknown;";
    let got = ok(C::StripTags, json!(src));
    assert_eq!(got, json!("caf\u{e9} & th\u{e9}xAB\u{a0}&unknown;"));
}

#[test]
fn parse_number_currency_and_disambiguation() {
    assert_eq!(
        ok(C::ParseNumber { as_int: false }, json!("$ 1,234.50")),
        json!(1234.5)
    );
    // European: dot thousands, comma decimal
    assert_eq!(
        ok(C::ParseNumber { as_int: false }, json!("1.234,50 \u{20ac}")),
        json!(1234.5)
    );
    // lone "x,ddd" with three trailing digits = thousands, not decimal
    assert_eq!(
        ok(C::ParseNumber { as_int: false }, json!("1,234")),
        json!(1234.0)
    );
    // lone "x.dd" two decimals = decimal point
    assert_eq!(
        ok(C::ParseNumber { as_int: false }, json!("9.95")),
        json!(9.95)
    );
    // accounting negatives
    assert_eq!(
        ok(C::ParseNumber { as_int: false }, json!("(42.50)")),
        json!(-42.5)
    );
    assert_eq!(
        ok(C::ParseNumber { as_int: true }, json!("19.99")),
        json!(20)
    );
    assert_eq!(
        ok(C::ParseNumber { as_int: true }, json!("-19.99")),
        json!(-20)
    );
    let err = one(C::ParseNumber { as_int: false }, json!("free!")).unwrap_err();
    assert!(err.contains("parse_number"), "{err}");
    // already-numeric input passes untouched (exact, no text round-trip)
    assert_eq!(ok(C::ParseNumber { as_int: false }, json!(7)), json!(7));
    assert_eq!(ok(C::ParseNumber { as_int: true }, json!(7.9)), json!(8));
    assert_eq!(
        ok(C::ParseNumber { as_int: false }, json!(1.005)),
        json!(1.005)
    );
}

#[test]
fn round_to_uses_ties_even() {
    assert_eq!(ok(C::RoundTo { decimals: 0 }, json!("2.5")), json!(2));
    assert_eq!(ok(C::RoundTo { decimals: 0 }, json!("3.5")), json!(4));
    assert_eq!(
        ok(C::RoundTo { decimals: 0 }, json!("1,234.50")),
        json!(1234)
    );
    // JSON numbers round from their exact binary value: 1.005 is stored
    // as 1.00499999... so it goes DOWN, 1.015 -> 1.01499... down too.
    // Text "1.005" would mean 1005 (thousands rule) — numbers skip it.
    assert_eq!(ok(C::RoundTo { decimals: 2 }, json!(1.005)), json!(1.0));
    assert_eq!(ok(C::RoundTo { decimals: 2 }, json!(1.015)), json!(1.01));
    assert_eq!(ok(C::RoundTo { decimals: 2 }, json!(1.025)), json!(1.02));
}

#[test]
fn units_bytes_binary_vs_decimal_are_distinct() {
    assert_eq!(
        ok(C::Units { to: "bytes".into() }, json!("2 KB")),
        json!(2000)
    );
    assert_eq!(
        ok(C::Units { to: "bytes".into() }, json!("2 KiB")),
        json!(2048)
    );
    assert_eq!(
        ok(C::Units { to: "bytes".into() }, json!("1,024 KB")),
        json!(1024000)
    );
    assert_eq!(
        ok(C::Units { to: "bytes".into() }, json!("512")),
        json!(512)
    );
    // thousands heuristic applies to text only...
    assert_eq!(
        ok(C::ParseNumber { as_int: false }, json!("1.005")),
        json!(1005.0)
    );
    // ...and `1,024 KB` is unambiguous (comma group + explicit unit)
    let err = one(C::Units { to: "bytes".into() }, json!("5 bananas")).unwrap_err();
    assert!(err.contains("unknown unit"), "{err}");
}

#[test]
fn units_seconds_and_percent() {
    assert_eq!(
        ok(
            C::Units {
                to: "seconds".into()
            },
            json!("90 min")
        ),
        json!(5400)
    );
    assert_eq!(
        ok(
            C::Units {
                to: "seconds".into()
            },
            json!("1.5h")
        ),
        json!(5400)
    );
    assert_eq!(
        ok(
            C::Units {
                to: "seconds".into()
            },
            json!("500ms")
        ),
        json!(0.5)
    );
    assert_eq!(
        ok(
            C::Units {
                to: "percent".into()
            },
            json!("12.5%")
        ),
        json!(0.125)
    );
    // bare number under percent is ambiguous by design -> error
    assert!(one(
        C::Units {
            to: "percent".into()
        },
        json!("12.5")
    )
    .is_err());
}

#[test]
fn parse_date_all_input_shapes_agree() {
    let want = json!("2015-10-21T07:28:00Z");
    assert_eq!(
        ok(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!("2015-10-21T07:28:00Z")
        ),
        want
    );
    assert_eq!(
        ok(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!("2015-10-21T09:28:00+02:00")
        ),
        want
    );
    assert_eq!(
        ok(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!("Wed, 21 Oct 2015 07:28:00 GMT")
        ),
        want
    );
    assert_eq!(
        ok(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!("1445412480")
        ),
        want
    );
    assert_eq!(
        ok(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!("1445412480000")
        ),
        want
    );
    assert_eq!(
        ok(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!("10/21/2015")
        ),
        json!("2015-10-21T00:00:00Z")
    );
    assert_eq!(
        ok(
            C::ParseDate {
                output: "unix".into()
            },
            json!("2015-10-21")
        ),
        json!(1445385600)
    );
    assert!(one(
        C::ParseDate {
            output: "rfc3339".into()
        },
        json!("somewhen")
    )
    .is_err());
    assert!(
        one(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!("2015-13-21")
        )
        .is_err(),
        "month 13 invalid"
    );
    assert!(
        one(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!("02/30/2015")
        )
        .is_err(),
        "US day 30 invalid"
    );
}

#[test]
fn rfc3339_seconds_only_normalization_stable_across_leap_years() {
    // 2016 was a leap year: Feb 29 legal, round-trips exactly
    assert_eq!(
        ok(
            C::ParseDate {
                output: "unix".into()
            },
            json!("2016-02-29T00:00:00Z")
        ),
        json!(1456704000)
    );
    assert_eq!(
        ok(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!(1456704000i64)
        ),
        json!("2016-02-29T00:00:00Z")
    );
    // pre-epoch: negative seconds format on the proleptic calendar
    assert_eq!(
        ok(
            C::ParseDate {
                output: "rfc3339".into()
            },
            json!(-1i64)
        ),
        json!("1969-12-31T23:59:59Z")
    );
}

#[test]
fn to_bool_defaults_and_custom_vocabulary() {
    assert_eq!(
        ok(
            C::ToBool {
                true_values: vec![],
                false_values: vec![]
            },
            json!(" Yes ")
        ),
        json!(true)
    );
    assert_eq!(
        ok(
            C::ToBool {
                true_values: vec![],
                false_values: vec![]
            },
            json!("OUT OF STOCK")
        ),
        json!(false)
    );
    assert_eq!(
        ok(
            C::ToBool {
                true_values: vec!["A".into()],
                false_values: vec!["B".into()]
            },
            json!("a")
        ),
        json!(true)
    );
    assert!(one(
        C::ToBool {
            true_values: vec!["A".into()],
            false_values: vec!["B".into()]
        },
        json!("yes")
    )
    .is_err());
}

#[test]
fn to_bool_overlap_and_empty_regexes_rejected_at_compile() {
    let mk = |transforms: Vec<C>| {
        let mut rules = BTreeMap::new();
        rules.insert(
            "f".to_string(),
            FieldCfg {
                css: "x".into(),
                path: None,
                attr: None,
                all: false,
                transforms,
            },
        );
        Pipeline::compile(&rules)
    };
    let err = mk(vec![C::ToBool {
        true_values: vec!["X".into()],
        false_values: vec!["x".into()],
    }])
    .unwrap_err()
    .to_string();
    assert!(err.contains("overlap"), "{err}");
    assert!(mk(vec![C::Split {
        sep: String::new(),
        trim: false,
        max: 0
    }])
    .is_err());
    assert!(mk(vec![C::Units {
        to: "furlongs".into()
    }])
    .is_err());
    assert!(mk(vec![C::ParseDate {
        output: "excel".into()
    }])
    .is_err());
    assert!(mk(vec![C::MaskPii {
        strategy: MaskStrategy::Redact,
        kinds: vec!["ssn".into()]
    }])
    .is_err());
}

#[test]
fn split_join_and_list_ops() {
    assert_eq!(
        ok(
            C::Split {
                sep: ",".into(),
                trim: true,
                max: 0
            },
            json!("a,b, c")
        ),
        json!(["a", "b", "c"])
    );
    assert_eq!(
        ok(
            C::Split {
                sep: ",".into(),
                trim: false,
                max: 2
            },
            json!("a,b,c")
        ),
        json!(["a", "b,c"])
    );
    assert_eq!(
        ok(C::Join { sep: " | ".into() }, json!(["a", "b"])),
        json!("a | b")
    );
    assert_eq!(
        ok(C::Unique, json!(["a", "b", "a", "b", "c"])),
        json!(["a", "b", "c"])
    );
    assert_eq!(ok(C::Sort, json!(["b", "c", "a"])), json!(["a", "b", "c"]));
    assert_eq!(ok(C::Take { count: 2 }, json!([1, 2, 3, 4])), json!([1, 2]));
    assert_eq!(ok(C::Drop { count: 2 }, json!([1, 2, 3, 4])), json!([3, 4]));
    assert_eq!(
        ok(C::Take { count: 9 }, json!([1])),
        json!([1]),
        "take past end is not an error"
    );
    assert!(one(C::Sort, json!("scalar")).is_err());
    assert!(one(C::Join { sep: ",".into() }, json!("scalar")).is_err());
}

#[test]
fn truncate_counts_chars_not_bytes() {
    // 4 chars, 7 bytes: cutting at 2 keeps whole codepoints
    assert_eq!(
        ok(C::Truncate { max_chars: 1 }, json!("caf\u{e9}s")),
        json!("c\u{2026}")
    );
    assert_eq!(
        ok(C::Truncate { max_chars: 2 }, json!("caf\u{e9}s")),
        json!("ca\u{2026}")
    );
    // 2 kept chars + ellipsis marker = 3 chars, all codepoint-aligned
    assert_eq!(
        ok(C::Truncate { max_chars: 2 }, json!("caf\u{e9}s"))
            .as_str()
            .map(|s| s.chars().count()),
        Some(3)
    );
    assert_eq!(
        ok(C::Truncate { max_chars: 10 }, json!("short")),
        json!("short"),
        "no marker under cap"
    );
}

#[test]
fn replace_empty_prefix_suffix() {
    assert_eq!(
        ok(C::ReplaceEmpty { with: "n/a".into() }, json!("   ")),
        json!("n/a")
    );
    assert_eq!(
        ok(C::ReplaceEmpty { with: "n/a".into() }, json!("x")),
        json!("x")
    );
    assert_eq!(
        ok(
            C::Prefix {
                with: "sku:".into()
            },
            json!("7")
        ),
        json!("sku:7")
    );
    assert_eq!(
        ok(C::Suffix { with: " cm".into() }, json!("30")),
        json!("30 cm")
    );
}

#[test]
fn luhn_accepts_only_checksum_valid_digit_runs() {
    assert!(luhn_ok("4242 4242 4242 4242"));
    assert!(luhn_ok("4242-4242-4242-4242"));
    assert!(!luhn_ok("4242 4242 4242 4241"), "one bad check digit");
    assert!(
        !luhn_ok("123456789012345678901"),
        "21 digits is out of band"
    );
}

#[test]
fn mask_pii_redact_keeps_head_and_tail() {
    let cfg = C::MaskPii {
        strategy: MaskStrategy::Redact,
        kinds: vec![],
    };
    let got = ok(cfg, json!("contact john.doe@example.com or 12.34.56.78"));
    let s = got.as_str().unwrap();
    assert!(s.starts_with("contact jo"), "{s}");
    assert!(s.contains("***"), "{s}");
    assert!(!s.contains("john.doe@example.com"), "email leaked: {s}");
    assert!(!s.contains("12.34.56.78"), "ip leaked: {s}");
}

#[test]
fn mask_pii_hash_is_stable_and_reversible_proof_impossible() {
    let cfg = C::MaskPii {
        strategy: MaskStrategy::Hash,
        kinds: vec!["email".into()],
    };
    let a = ok(cfg.clone(), json!("a@x.io"));
    let b = ok(cfg, json!("a@x.io"));
    assert_eq!(a, b, "hash must be deterministic for joins");
    let s = a.as_str().unwrap();
    assert!(s.starts_with("a@#") && s.len() == "a@#".len() + 8, "{s}");
}

#[test]
fn mask_pii_drop_and_ipv4_octet_guard() {
    let cfg = C::MaskPii {
        strategy: MaskStrategy::Drop,
        kinds: vec!["ipv4".into()],
    };
    assert_eq!(
        ok(cfg.clone(), json!("server 10.0.0.1 ok")),
        json!("server  ok")
    );
    assert_eq!(
        ok(cfg, json!("version 999.1.1.1")),
        json!("version 999.1.1.1"),
        "octets >255 are not IPs"
    );
}

#[test]
fn mask_pii_card_requires_luhn_and_phone_needs_digit_band() {
    let cfg = C::MaskPii {
        strategy: MaskStrategy::Redact,
        kinds: vec!["card".into(), "phone".into()],
    };
    let got = ok(cfg.clone(), json!("pay 4242 4242 4242 4242 now"));
    assert!(
        !got.as_str().unwrap().contains("4242 4242"),
        "card leaked: {got}"
    );
    // Not-Luhn 16 digits stays intact (it is an order id, not a PAN)
    let keep = ok(cfg.clone(), json!("order 4242 4242 4242 4241"));
    assert!(keep.as_str().unwrap().contains("4242 4242 4242 4241"));
    let ph = ok(cfg.clone(), json!("call +1 (555) 010-9988 asap"));
    assert!(
        !ph.as_str().unwrap().contains("010-9988"),
        "phone leaked: {ph}"
    );
    // 5 digits is not a phone; price noise survives
    let no = ok(cfg, json!("id 12345"));
    assert_eq!(no, json!("id 12345"));
}

#[test]
fn chains_are_atomic_original_value_survives_failure() {
    let mut rules = BTreeMap::new();
    rules.insert(
        "f".to_string(),
        FieldCfg {
            css: "x".into(),
            path: None,
            attr: None,
            all: false,
            transforms: vec![C::Uppercase, C::ParseNumber { as_int: false }],
        },
    );
    let p = Pipeline::compile(&rules).unwrap();
    let mut fields = Map::new();
    fields.insert("f".into(), json!("abc"));
    let issues = p.apply(&mut fields);
    assert_eq!(issues.len(), 1);
    assert_eq!(fields["f"], json!("abc"), "no half-transformed fields");
}

#[test]
fn identity_pipeline_is_empty_and_skips() {
    let mut rules = BTreeMap::new();
    rules.insert(
        "f".to_string(),
        FieldCfg {
            css: "x".into(),
            path: None,
            attr: None,
            all: false,
            transforms: vec![],
        },
    );
    let p = Pipeline::compile(&rules).unwrap();
    assert!(p.is_identity());
    assert!(!p.has_field("f"));
}

#[test]
fn civil_algorithm_matches_known_days() {
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(days_from_civil(2000, 3, 1), 11017);
    assert_eq!(
        days_from_civil(1900, 3, 2),
        -25507,
        "1900 is NOT a leap year"
    );
    // round-trip through the rfc formatter at a pre-epoch date
    assert_eq!(
        epoch_rfc3339(days_from_civil(1969, 7, 20) * 86400),
        "1969-07-20T00:00:00Z"
    );
}
