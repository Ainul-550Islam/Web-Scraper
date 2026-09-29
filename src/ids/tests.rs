use super::*;

#[test]
fn slug_rules_are_exact() {
    assert!(TargetId::parse("products").is_ok());
    assert!(TargetId::parse("a").is_ok());
    assert!(TargetId::parse("run_42.v2-x").is_ok());
    for bad in [
        "",
        "..",
        "/",
        "a/b",
        "a\\b",
        "a:b",
        "A-bad",
        "-lead",
        ".lead",
        "a b",
        "a\nb",
        "\u{e9}uro",
        "end.",
        "a..b",
        ".",
    ] {
        assert!(TargetId::parse(bad).is_err(), "must reject {bad:?}");
    }
    let long = "x".repeat(64);
    assert!(TargetId::parse(&long).is_ok(), "boundary: 64 is fine");
    assert!(TargetId::parse(&format!("{long}y")).is_err(), "65 fails");
}

#[test]
fn ordering_is_lexicographic_and_stable() {
    // invalid names drop out of the parse set entirely ("Mike1" is not a
    // ProxyId); what remains sorts by plain bytes — stable across runs.
    let mut ids = ["zulu", "alpha", "Mike1", "alpha-x"]
        .iter()
        .filter_map(|s| ProxyId::parse(s).ok())
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(
        ids.iter().map(|i| i.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "alpha-x", "zulu"]
    );
}

#[test]
fn serde_round_trips_as_bare_string() {
    let id = TargetId::parse("catalog").unwrap();
    let j = serde_json::to_string(&id).unwrap();
    assert_eq!(j, "\"catalog\"");
    let back: TargetId = serde_json::from_str(&j).unwrap();
    assert_eq!(back, id);
    // invalid wire values are refused at deserialize time
    assert!(serde_json::from_str::<TargetId>("\"../evil\"").is_err());
}

#[test]
fn record_ids_are_sha256_hex_only() {
    let ok = "a".repeat(64);
    assert!(RecordId::parse(&ok).is_ok());
    for bad in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
        assert!(RecordId::parse(&bad).is_err(), "reject {bad:?}");
    }
    assert!(RecordId::parse("").is_err());
}

#[test]
fn lease_ids_validate_the_generated_shape() {
    let w = WorkerId::parse("box1").unwrap();
    let l = LeaseId::new(&w, 7, 0xdead_beef);
    assert_eq!(l.as_str(), "box1-s7-deadbeef");
    assert_eq!(LeaseId::parse(l.as_str()).unwrap(), l);
    for bad in [
        "box1-s7",
        "box1-sx-12345678",
        "nope",
        "box1-s7-zzzzzzzz",
        "BAD-worker-s1-00000000",
    ] {
        assert!(LeaseId::parse(bad).is_err(), "reject {bad:?}");
    }
}

#[test]
fn timestamps_and_partitions_agree() {
    // 2026-09-20 00:00:00 UTC = 1789862400? pin against civil math:
    let ts = Timestamp(1_789_862_400);
    let day = ts.utc_day();
    let parts = DatasetPartition::for_record_ts(1_789_862_400);
    assert_eq!(parts.dir_name(), format!("dt={day}"));
    assert_eq!(
        DatasetPartition::parse_selector(&format!("dt={day}")).unwrap(),
        parts
    );
    assert_eq!(
        DatasetPartition::parse_selector(&ts.utc_day_compact()).unwrap(),
        parts
    );
    assert!(DatasetPartition::parse_selector("dt=2026-13-01").is_err());
    assert!(DatasetPartition::parse_selector("parts").is_err());
}
