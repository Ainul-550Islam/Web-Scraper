//! Property-style invariant sweeps with a seeded deterministic PRNG (no new
//! dependency; a failing seed re-runs identically). These guard the algebraic
//! promises: ids round-trip, budget accounting is exact, chunking is
//! lossless, config migration is idempotent.

use std::sync::Arc;

use harvest::budget::Budget;
use harvest::ids::{RecordId, TargetId};
use harvest::migrate;

pub struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(2654435761) | 1)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let mut z = self.0;
        z = (z ^ (z >> 15)).wrapping_mul(z | 1);
        z = z.wrapping_add((z ^ (z >> 7)).wrapping_mul(z | 61));
        z ^ (z >> 14)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next_u64() & 0xff) as u8).collect()
    }
}

fn hex(s: &[u8]) -> String {
    s.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn record_ids_round_trip_and_reject_deformations() {
    let mut r = Rng::new(0xC0FFEE);
    for _ in 0..2000 {
        let fp = hex(&r.bytes(32));
        let p = RecordId::parse(&fp).expect("generated sha256 hex must parse");
        assert_eq!(p.as_str(), fp);
        let mut too_short = fp.clone();
        too_short.pop();
        assert!(RecordId::parse(&too_short).is_err());
        assert!(RecordId::parse(&fp.to_uppercase()).is_err());
        let mut swapped = fp.clone();
        swapped.replace_range(0..1, "g");
        assert!(RecordId::parse(&swapped).is_err());
    }
}

#[test]
fn target_id_accepts_only_its_declared_charset() {
    // seeded generation of the *good* class: every char in the alphabet,
    // every length at the boundaries.
    let mut r = Rng::new(7);
    for len in [1usize, 2, 32, 63, 64] {
        for _ in 0..200 {
            let s: String = (0..len)
                .map(|_| {
                    let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789-_";
                    alphabet[r.below(alphabet.len() as u64) as usize] as char
                })
                .collect();
            let s = if s.as_bytes()[0].is_ascii_alphabetic() || s.as_bytes()[0].is_ascii_digit() {
                s
            } else {
                format!("a{s}").chars().take(len).collect()
            };
            assert!(TargetId::parse(&s).is_ok(), "{s:?} must be valid");
        }
    }
    for good in ["a", "shop", "a1", "a-b", "a_b", "a.b", &"x".repeat(64), "9"] {
        assert!(TargetId::parse(good).is_ok(), "{good:?} must be valid");
    }
    for bad in [
        "",
        "A",
        "-lead",
        "_lead",
        ".",
        "..",
        "a b",
        "a/b",
        "a\\b",
        "a:b",
        "end.",
        "a..b",
        &"x".repeat(65),
        "a!b",
        "a\nb",
    ] {
        assert!(TargetId::parse(bad).is_err(), "{bad:?} must be invalid");
    }
}

#[test]
fn budget_holds_are_exact_under_arbitrary_interleaving() {
    for trial in 0..400u64 {
        let mut r = Rng::new(1000 + trial);
        let cap = r.below(64) + 1;
        let b = Arc::new(Budget::new("property", cap));
        let mut live: Vec<(usize, u64)> = Vec::new(); // (slot order, n)
        let mut used = 0u64;
        for _ in 0..500 {
            let want = r.below(8) + 1;
            if r.below(3) == 0 && !live.is_empty() {
                // release a random guard (drop is the release)
                let i = r.below(live.len() as u64) as usize;
                let (slot, n) = live.swap_remove(i);
                // guards live in a side vec indexed by slot; drop frees it
                GUARD.with(|g| drop(g.borrow_mut()[slot].take()));
                used -= n;
            }
            match b.try_hold(want) {
                Some(h) => {
                    let slot = {
                        GUARD.with(|g| {
                            let mut g = g.borrow_mut();
                            g.push(Some(h));
                            g.len() - 1
                        })
                    };
                    live.push((slot, want));
                    used += want;
                    assert!(used <= cap, "never over-commits");
                }
                None => {
                    assert!(
                        used + want > cap,
                        "refused while room existed ({used}+{want}<={cap})"
                    );
                    assert!(b.denied() >= 1);
                }
            }
            assert_eq!(b.used(), used, "accounting matches the ledger");
        }
        // drain: full capacity returns
        GUARD.with(|g| g.borrow_mut().clear());
        live.clear();
        assert_eq!(b.used(), 0, "no leak across the whole cycle");
        assert_eq!(b.view().limit, cap);
        assert_eq!(b.peak(), b.view().peak);
    }
}

thread_local! {
    static GUARD: std::cell::RefCell<Vec<Option<harvest::budget::BudgetHold>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[test]
fn export_chunking_is_lossless_for_arbitrary_caps() {
    use harvest::export::{self, ExportOpts};
    let dir = std::env::temp_dir().join(format!("harvest-props-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = Rng::new(5);
    let mut lines = String::new();
    let n = 24usize;
    for i in 0..n {
        let pad = "x".repeat(r.below(200) as usize);
        lines.push_str(
            &serde_json::json!({
                "run": "r", "target": "shop", "url": format!("https://x.test/{i}"),
                "depth": 0, "ts": i, "fingerprint": format!("{:064x}", i), "egress": "d",
                "fields": { "body": pad },
            })
            .to_string(),
        );
        lines.push('\n');
    }
    let input = dir.join("records.jsonl");
    std::fs::write(&input, &lines).unwrap();
    let plain = dir.join("plain");
    let opts = |out: &std::path::Path, cap: u64| ExportOpts {
        inputs: vec![input.clone()],
        out_dir: out.to_path_buf(),
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: cap,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    let full = export::run(&opts(&plain, 0)).unwrap();
    let want = std::fs::read_to_string(plain.join("records.jsonl")).unwrap();
    assert_eq!(full.manifest.records, n as u64);
    for trial in 0..25u64 {
        let mut r2 = Rng::new(500 + trial);
        let cap = r2.below(900) + 40;
        let out = dir.join(format!("cap-{trial}"));
        let rep = export::run(&opts(&out, cap)).unwrap();
        assert_eq!(
            rep.manifest.records, n as u64,
            "cap changes layout, not content"
        );
        let mut joined = String::new();
        for f in rep
            .manifest
            .files
            .iter()
            .filter(|f| f.name.ends_with(".jsonl"))
        {
            let body = std::fs::read_to_string(out.join(&f.name)).unwrap();
            assert!(
                body.len() as u64 <= cap || body.matches('\n').count() == 1,
                "part {} oversized ({}) at cap {cap}",
                f.name,
                body.len()
            );
            joined.push_str(&body);
        }
        assert_eq!(joined, want, "parts concatenate to the single-file export");
        assert!(
            export::verify(&out, None).unwrap().ok,
            "verify PASS at cap {cap}"
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_migration_is_idempotent_and_field_lossless() {
    let mut r = Rng::new(2026);
    for _ in 0..500 {
        let name: String = (0..6).map(|_| (b'a' + r.below(26) as u8) as char).collect();
        let mut doc = serde_json::json!({
            "name": name,
            "concurrency": r.below(500) + 1,
            "max_retries": r.below(20),
            "respect_robots": r.below(2) == 0,
        });
        if r.below(2) == 0 {
            doc["config_version"] = serde_json::json!(1);
        }
        let (m1, _w1) = migrate::migrate(&doc).unwrap();
        let (m2, w2) = migrate::migrate(&m1).unwrap();
        assert_eq!(m1, m2, "one migration pass reaches the fixed point");
        assert!(w2.is_empty(), "the fixed point must not re-warn: {w2:?}");
        assert_eq!(m1["config_version"], migrate::CURRENT_VERSION);
        assert_eq!(m1["name"], doc["name"]);
        assert_eq!(m1["concurrency"], doc["concurrency"]);
        assert_eq!(m1["respect_robots"], doc["respect_robots"]);
        let known: Vec<&str> = harvest::Config::TOP_LEVEL_KEYS.to_vec();
        assert!(migrate::check_unknown_keys(&m1, &known).is_ok());
        doc["cuncurrancy"] = serde_json::json!(3);
        let Err(e) = migrate::check_unknown_keys(&doc, &known) else {
            panic!("unknown key must be refused");
        };
        assert!(e.contains("cuncurrancy"), "{e}");
    }
}

#[test]
fn future_stamps_fail_closed_never_silently_downgrade() {
    let mut r = Rng::new(99);
    for _ in 0..200 {
        let v = migrate::CURRENT_VERSION + 1 + (r.below(1000) as u32);
        let doc = serde_json::json!({"config_version": v, "name": "x"});
        let Err(e) = migrate::migrate(&doc) else {
            panic!("future config v{v} must be refused");
        };
        assert!(e.contains(&v.to_string()) || e.contains("newer"), "{e}");
    }
}
