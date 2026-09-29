use super::*;

use serde_json::json;

#[test]
fn v1_files_migrate_with_only_a_stamp() {
    let v1 = json!({"name": "x", "targets": []});
    let (doc, warnings) = migrate(&v1).unwrap();
    assert_eq!(doc["config_version"], CURRENT_VERSION);
    assert_eq!(doc["name"], "x");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("no field changes"));
    // the stamp leads the document (file hygiene for humans)
    assert_eq!(
        doc.as_object().unwrap().keys().next().unwrap(),
        "config_version"
    );
    // migration is idempotent: running it on the output changes nothing
    let (again, w2) = migrate(&doc).unwrap();
    assert_eq!(again, doc);
    assert!(w2.is_empty(), "already-current files get no warnings");
}

#[test]
fn future_versions_fail_hard_and_actionably() {
    let future = json!({"config_version": 99, "name": "x"});
    let e = migrate(&future).unwrap_err();
    assert!(e.contains("newer than this build"), "{e}");
    assert!(e.contains("99"), "names the version: {e}");
}

#[test]
fn version_gaps_are_refused_not_skipped() {
    // A table without 0→1 means v0 must error, NOT silently chain.
    let doc = json!({"config_version": 0});
    // v0 → CURRENT requires the 0→1 row; the table starts at 1→2.
    let err = migrate(&doc).expect_err("v0 has no migration path");
    assert!(err.contains("no migration path"), "{err}");
}

#[test]
fn unknown_keys_are_caught_with_suggestions() {
    let known = ["name", "targets", "max_retries", "respect_robots"];
    let good = json!({"name": "a", "targets": []});
    assert!(check_unknown_keys(&good, &known).is_ok());
    let typo = json!({"name2": "a"});
    let e = check_unknown_keys(&typo, &known).unwrap_err();
    assert!(e.contains("unknown config key `name2`"), "{e}");
    assert!(e.contains("did you mean `name`"), "the hint fired: {e}");
    let commented = json!({"_note": "human annotation", "name": "a"});
    assert!(
        check_unknown_keys(&commented, &known).is_ok(),
        "_-keys are comments"
    );
    let junk = json!({"totally_unrelated_deep_key": 1});
    let e = check_unknown_keys(&junk, &known).unwrap_err();
    assert!(!e.contains("did you mean"), "no false hints: {e}");
}

#[test]
fn edit1_detects_the_typo_classes() {
    assert!(edit1("name", "name"));
    assert!(edit1("name", "nane")); // substitution
    assert!(edit1("name", "names")); // insertion
    assert!(edit1("names", "name")); // deletion
    assert!(!edit1("name", "other")); // far apart
    assert!(!edit1("ab", "xy")); // two substitutions
}

#[test]
fn rewritten_file_is_valid_json_with_the_stamp() {
    let s = rewritten_file(&json!({"name": "q"}));
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["config_version"], CURRENT_VERSION);
    assert!(s.ends_with("}\n"));
}
