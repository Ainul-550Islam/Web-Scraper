use super::*;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-reval-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(&d).unwrap();
    d.join("validators.jsonl")
}

#[test]
fn round_trip_with_ttl_and_garbage_tolerance() {
    let p = tmp("rt");
    let _ = std::fs::remove_file(&p);
    {
        let s = Store::load(p.clone(), 0).unwrap();
        s.note_200(
            "h/x",
            Some("\"abc\"".into()),
            Some("Tue, 15 Nov 2022 08:00:00 GMT".into()),
            vec!["/a".into()],
        );
        s.note_200("h/y", None, None, vec![]); // nothing useful => not saved
        s.save().unwrap();
    }
    {
        let s = Store::load(p.clone(), 0).unwrap();
        assert_eq!(s.len(), 1, "useless entry pruned on save");
        match s.condition_for("h/x") {
            Condition::Headers(h) => {
                assert_eq!(h[0].0, "if-none-match");
                assert_eq!(h[0].1, "\"abc\"");
                assert_eq!(h[1].0, "if-modified-since");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(s.condition_for("h/missing"), Condition::None);
    }
    // corrupt line tolerance: append junk, reload keeps the good entry
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{{broken json").unwrap();
    }
    let s = Store::load(p.clone(), 0).unwrap();
    assert_eq!(s.len(), 1);
    std::fs::remove_file(&p).ok();
}

#[test]
fn expiry_link_cap_and_touch() {
    // TTL expiry
    let s = Store::load(tmp("ttl"), 1).unwrap();
    s.note_200("k", Some("\"e\"".into()), None, vec![]);
    assert!(matches!(s.condition_for("k"), Condition::Headers(_)));
    {
        let mut g = s.map.lock().unwrap();
        g.get_mut("k").unwrap().ts -= 5; // pretend 5s old
    }
    assert_eq!(s.condition_for("k"), Condition::Expired);
    assert_eq!(s.condition_for("never-seen"), Condition::None);

    // LINK_CAP truncation on write
    let many: Vec<String> = (0..LINK_CAP + 50).map(|i| format!("/l{i}")).collect();
    let p = tmp("cap");
    let _ = std::fs::remove_file(&p);
    {
        let s = Store::load(p.clone(), 0).unwrap();
        s.note_200("big", Some("\"e\"".into()), None, many);
        let g = s.map.lock().unwrap();
        assert_eq!(g.get("big").unwrap().links.len(), LINK_CAP);
        assert_eq!(
            g.get("big").unwrap().links[LINK_CAP - 1],
            format!("/l{}", LINK_CAP - 1)
        );
    }
    std::fs::remove_file(&p).ok();

    // touch refreshes freshness window
    let s = Store::load(tmp("touch"), 30).unwrap();
    s.note_200("t", Some("\"e\"".into()), None, vec![]);
    {
        let mut g = s.map.lock().unwrap();
        g.get_mut("t").unwrap().ts -= 100;
    }
    assert_eq!(s.condition_for("t"), Condition::Expired);
    s.touch("t");
    assert!(matches!(s.condition_for("t"), Condition::Headers(_)));
}

#[test]
fn link_set_is_capped_and_in_memory_absence_is_not_a_violation() {
    let p = tmp("cap");
    let _ = std::fs::remove_file(&p);
    let s = Store::load(p.clone(), 0).unwrap();
    let many: Vec<String> = (0..LINK_CAP + 7).map(|i| format!("/l{i}")).collect();
    s.note_200("big", Some("\"e\"".into()), None, many);
    let snap = s.snapshot("big").unwrap();
    assert_eq!(
        snap.links.len(),
        LINK_CAP,
        "nav-heavy pages must not balloon the store"
    );
    assert_eq!(
        snap.links.first().unwrap(),
        "/l0",
        "cap keeps the FIRST links"
    );
    // entry with no validators at all: plain fetch, not a conditional
    s.note_200("bare", None, None, vec!["/x".into()]);
    assert_eq!(s.condition_for("bare"), Condition::None);
    // lm-only entry: single header, correct name
    s.note_200(
        "lm",
        None,
        Some("Tue, 15 Nov 2022 08:00:00 GMT".into()),
        vec![],
    );
    match s.condition_for("lm") {
        Condition::Headers(h) => {
            assert_eq!(h.len(), 1);
            assert_eq!(h[0].0, "if-modified-since");
        }
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_file(&p);
}

#[test]
fn ttl_expiry_is_reported_as_expired_not_none() {
    // Hand-age the file: ts=1 with ttl 60 => Expired (fetch for real).
    let p = tmp("ttl");
    let st = UrlState {
        etag: Some("\"v1\"".into()),
        lm: None,
        links: vec![],
        ts: 1,
    };
    let line = serde_json::to_string(&("http://x/".to_string(), st)).unwrap();
    std::fs::write(&p, format!("{{\"v\":1}}\n{line}\n")).unwrap();
    let s = Store::load(p.clone(), 60).unwrap();
    assert_eq!(s.condition_for("http://x/"), Condition::Expired);
    // same entry with ttl disabled (0): the validator is used anyway
    let s0 = Store::load(p.clone(), 0).unwrap();
    assert!(matches!(
        s0.condition_for("http://x/"),
        Condition::Headers(_)
    ));
    // unknown keys are never Expired, they are simply None
    assert_eq!(s.condition_for("http://never-seen/"), Condition::None);
    let _ = std::fs::remove_file(&p);
}
