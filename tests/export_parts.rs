//! Chunked exports (record-boundary parts), crash-resume semantics, and
//! `harvest verify` — the delivery-integrity layer.

use harvest::export::{self, ExportOpts};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture(dir: &Path, n: usize) -> PathBuf {
    let mut lines = String::new();
    for i in 0..n {
        let fp = format!("{i:064x}");
        lines.push_str(
            &json!({
                "run": "r1", "target": if i % 2 == 0 { "shop" } else { "blog" },
                "url": format!("https://x.test/p/{i}"), "depth": 0, "ts": 1700000000 + i as i64,
                "fingerprint": fp, "egress": "direct",
                "fields": { "title": format!("page number {i} with some padding text"), "price": i },
            })
            .to_string(),
        );
        lines.push('\n');
    }
    let p = dir.join("records.jsonl");
    fs::write(&p, lines).unwrap();
    p
}

fn opts(inputs: Vec<PathBuf>, out: &Path, cap: u64, resume: bool) -> ExportOpts {
    ExportOpts {
        inputs,
        out_dir: out.to_path_buf(),
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: cap,
        resume,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    }
}

#[test]
fn chunking_cuts_at_record_boundaries_and_manifest_covers_all_parts() {
    let dir = tempfile();
    let input = fixture(&dir, 40);
    let out = dir.join("exp");
    let cap = 400;
    let r = export::run(&opts(vec![input.clone()], &out, cap, false)).unwrap();
    let jsonl_parts: Vec<_> = r
        .manifest
        .files
        .iter()
        .filter(|f| f.name.ends_with(".jsonl"))
        .collect();
    assert!(
        jsonl_parts.len() > 4,
        "40 records at 400B/part must chunk: {} parts",
        jsonl_parts.len()
    );
    let mut total_lines = 0u64;
    for f in &jsonl_parts {
        let body = fs::read_to_string(out.join(&f.name)).unwrap();
        assert!(
            body.len() as u64 <= cap,
            "{} exceeds the cap: {} > {cap}",
            f.name,
            body.len()
        );
        for line in body.lines() {
            // every line is a whole record — chunking never cuts JSON
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
        total_lines += f.lines;
    }
    assert_eq!(total_lines, r.manifest.records);
    // each csv part is self-describing: header in every part
    let csv_part = out.join(
        r.manifest
            .files
            .iter()
            .find(|f| f.name.ends_with(".csv"))
            .unwrap()
            .name
            .clone(),
    );
    assert!(fs::read_to_string(csv_part)
        .unwrap()
        .starts_with("run,target,url"));
    // concatenating parts in name order reproduces the single-file export
    let plain = dir.join("exp-plain");
    let p = export::run(&opts(vec![input], &plain, 0, false)).unwrap();
    let mut joined = String::new();
    for f in &jsonl_parts {
        joined.push_str(&fs::read_to_string(out.join(&f.name)).unwrap());
    }
    assert_eq!(
        joined,
        fs::read_to_string(plain.join("records.jsonl")).unwrap()
    );
    let _ = p;
    assert!(export::verify(&out, None).unwrap().ok);
    assert!(export::verify(&plain, None).unwrap().ok);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn resume_survives_a_crash_mid_finalize_and_reuses_valid_parts() {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile();
    let input = fixture(&dir, 30);
    let out = dir.join("exp");
    // Force the first run to die during finalize: occupy one part name with
    // a directory so the staging->out rename fails there.
    fs::create_dir_all(&out).unwrap();
    let victim = "records.part-00002.jsonl";
    fs::create_dir(out.join(victim)).unwrap();
    let first = export::run(&opts(vec![input.clone()], &out, 350, false));
    assert!(
        first.is_err(),
        "finalize must refuse to clobber a directory"
    );
    // progress sidecar exists for resume; manifest must NOT
    assert!(out.join(export::PROGRESS_FILE).exists());
    assert!(!out.join("manifest.json").exists());
    let done_before: Vec<PathBuf> = fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy() == "records.part-00001.jsonl")
        .collect();
    assert_eq!(
        done_before.len(),
        1,
        "the part before the victim was finalized"
    );
    #[cfg(unix)]
    let ino_before = fs::metadata(&done_before[0]).unwrap().ino();
    fs::remove_dir(out.join(victim)).unwrap(); // fix the hazard
    let second = export::run(&opts(vec![input], &out, 350, true)).unwrap();
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&done_before[0]).unwrap().ino(),
        ino_before,
        "resume must not rewrite a part whose sha still validates"
    );
    assert!(
        !out.join(export::PROGRESS_FILE).exists(),
        "sidecar removed on finalize"
    );
    let v = export::verify(&out, None).unwrap();
    assert!(v.ok, "verified: {v:?}");
    // identical content to a fresh uninterrupted run
    let plain = dir.join("fresh");
    let fresh = export::run(&opts(vec![dir.join("records.jsonl")], &plain, 350, false)).unwrap();
    let names = |m: &export::Manifest| {
        m.files
            .iter()
            .filter(|f| f.name.ends_with(".jsonl"))
            .map(|f| (f.name.clone(), f.sha256.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&second.manifest), names(&fresh.manifest));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn verify_detects_tampering_reports_extras_and_checks_signatures() {
    let dir = tempfile();
    let input = fixture(&dir, 8);
    let out = dir.join("exp");
    // unsigned first: signature note is informational
    export::run(&opts(vec![input.clone()], &out, 0, false)).unwrap();
    let v = export::verify(&out, None).unwrap();
    assert!(v.ok && v.signature == "unsigned" && v.extras.is_empty());
    // extra file: reported, doesn't fail integrity
    fs::write(out.join("readme.txt"), "hi").unwrap();
    let v = export::verify(&out, None).unwrap();
    assert!(v.ok && v.extras == vec!["readme.txt".to_string()]);
    fs::remove_file(out.join("readme.txt")).unwrap();
    // flip one byte inside a record: sha fails even though bytes match
    let p = out.join("records.jsonl");
    let mut body = fs::read_to_string(&p).unwrap();
    body.pop();
    body.push(' ');
    fs::write(&p, &body).unwrap();
    let v = export::verify(&out, None).unwrap();
    assert!(!v.ok);
    let fc = &v.files.iter().find(|f| f.name == "records.jsonl").unwrap();
    assert!(fc.present && fc.bytes_ok && !fc.sha_ok);
    // signed export: verify takes a key, tampering still caught after re-export
    fs::remove_dir_all(&out).unwrap();
    let key = dir.join("attest.key");
    harvest::sign::Key::generate(&key).unwrap();
    let mut o = opts(vec![input], &out, 0, false);
    o.key = Some(key.clone());
    let r = export::run(&o).unwrap();
    assert!(r.manifest_sig.is_some());
    let v = export::verify(&out, Some(&key)).unwrap();
    assert!(v.ok && v.signature == "valid", "{v:?}");
    // tamper the MANIFEST itself: signature check catches it
    let mp = out.join("manifest.json");
    let mut mtext = fs::read_to_string(&mp).unwrap();
    mtext = mtext
        .replace("\"records\":8", "\"records\":7")
        .replace("\"records\": 8", "\"records\": 7");
    fs::write(&mp, mtext).unwrap();
    let v = export::verify(&out, Some(&key)).unwrap();
    assert!(!v.ok && v.signature.starts_with("invalid"), "{v:?}");
    // missing manifest: clean structural error, not a panic
    fs::remove_file(&mp).unwrap();
    let v = export::verify(&out, None).unwrap();
    assert!(!v.ok && v.error.as_deref().unwrap().contains("no manifest.json"));
    fs::remove_dir_all(&dir).ok();
}

fn tempfile() -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("harvest-export-parts-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}
