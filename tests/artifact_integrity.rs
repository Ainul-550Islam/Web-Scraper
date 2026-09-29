//! Artifact integrity: export → manifest → verify → classify, including
//! tamper, missing and partial scenarios. Uses the REAL `harvest::export`
//! implementation (no fixture manifests) so the typed artifact view in
//! `harvest::artifact` is proven against the bytes the engine actually ships.

use std::io::Write;
use std::path::{Path, PathBuf};

use harvest::artifact::{self, ValidationState};
use harvest::export::{self, ExportOpts};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(tag: &str) -> Dir {
    let mut p = std::env::temp_dir();
    p.push(format!("harvest-art-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    Dir(p)
}

fn sample_jsonl(dir: &Dir) -> PathBuf {
    let src = dir.0.join("basic.jsonl");
    let mut f = std::fs::File::create(&src).unwrap();
    for (url, title) in [
        ("http://127.0.0.1:8099/index.html", "Catalog Home"),
        ("http://127.0.0.1:8099/p/1.html", "Widget"),
        ("http://127.0.0.1:8099/p/2.html", "Widget"), // dup url ⇒ unique_urls < records
    ] {
        writeln!(
            f,
            "{}",
            serde_json::json!({
                "run": "art", "target": "shop", "url": url, "depth": 0, "ts": 1,
                "fingerprint": format!("{:0>64}", url.len()), "egress": "direct",
                "fields": { "title": title }
            })
        )
        .unwrap();
    }
    src
}

fn report_sig_none(out: &Path) -> bool {
    !out.join("manifest.json.sig").exists()
}

fn do_export(dir: &Dir, key: Option<&Path>) -> PathBuf {
    let out = dir.0.join("deliver");
    let report = export::run(&ExportOpts {
        inputs: vec![sample_jsonl(dir)],
        out_dir: out.clone(),
        split_target: false,
        history: None,
        key: key.map(|p| p.to_path_buf()),
        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    })
    .expect("export runs");
    assert_eq!(report.manifest.records, 3);
    out
}

fn write_key(dir: &Dir) -> PathBuf {
    let k = dir.0.join("attest.key");
    // matches `harvest attest --gen-key` output format: 32 raw hex bytes? No:
    // sign::Key expects 64 hex chars (32-byte key). Pin the format here.
    std::fs::write(&k, "ab".repeat(32)).unwrap();
    k
}

fn verify(dir: &Path, key: Option<&Path>) -> export::Verification {
    export::verify(dir, key).expect("verify executes (its verdict is the claim)")
}

#[test]
fn clean_delivery_classifies_verified_unsigned() {
    let d = tmp("clean");
    let out = do_export(&d, None);
    let art = artifact::load(&out).expect("manifest parses into the typed view");
    assert_eq!(art.identity.records, 3);
    assert_eq!(
        art.identity.unique_urls, 3,
        "URL accounting comes from the manifest itself"
    );
    assert_eq!(
        art.files.len(),
        3,
        "manifest promises the payload pair plus checksums.txt (it does not self-list)"
    );
    assert!(
        art.files.iter().any(|f| f.name == "checksums.txt"),
        "checksums.txt is part of every delivery"
    );
    let v = verify(&out, None);
    assert!(v.ok);
    assert!(report_sig_none(&out));
    assert_eq!(
        artifact::classify(&v),
        ValidationState::VerifiedUnsigned,
        "no key, no sig file ⇒ `unsigned` per the verify vocabulary"
    );
}

#[test]
fn signed_delivery_round_trips_to_verified_signed() {
    let d = tmp("signed");
    let key = write_key(&d);
    let out = do_export(&d, Some(&key));
    assert!(
        out.join("manifest.json.sig").exists(),
        "detached sig is written"
    );
    let v = verify(&out, Some(key.as_path()));
    assert_eq!(v.signature, "valid");
    assert!(v.ok);
    assert_eq!(artifact::classify(&v), ValidationState::VerifiedSigned);
    assert!(artifact::classify(&v).is_ok());
}

#[test]
fn one_byte_flip_is_tampered_not_silently_ok() {
    let d = tmp("tamper");
    let out = do_export(&d, None);
    let data = out.join("records.jsonl");
    let mut bytes = std::fs::read(&data).unwrap();
    let pos = bytes
        .iter()
        .position(|b| *b == b't')
        .expect("some byte to flip");
    bytes[pos] = b'T';
    std::fs::write(&data, &bytes).unwrap();
    let v = verify(&out, None);
    assert!(!v.ok, "checksum promise broken ⇒ verify fails");
    assert!(v.files.iter().any(|f| f.present && !f.sha_ok));
    assert_eq!(artifact::classify(&v), ValidationState::Tampered);
}

#[test]
fn missing_payload_is_partial() {
    let d = tmp("partial");
    let out = do_export(&d, None);
    std::fs::remove_file(out.join("records.csv")).unwrap();
    let v = verify(&out, None);
    assert!(!v.ok);
    assert!(v.files.iter().any(|f| !f.present));
    assert_eq!(artifact::classify(&v), ValidationState::Partial);
}

#[test]
fn missing_manifest_short_circuits_everything() {
    let d = tmp("nomanifest");
    let out = do_export(&d, None);
    std::fs::remove_file(out.join("manifest.json")).unwrap();
    let v = verify(&out, None);
    assert!(
        v.error.is_some(),
        "verify reports the fatal missing manifest"
    );
    assert_eq!(artifact::classify(&v), ValidationState::MissingManifest);
    // artifact::load refuses too, with a structured error
    assert!(matches!(
        artifact::load(&out),
        Err(artifact::ArtifactError::Unreadable(_)) | Err(artifact::ArtifactError::NotFound(_))
    ));
}

#[test]
fn signature_checked_against_wrong_key_is_invalid() {
    let d = tmp("wrongkey");
    let key = write_key(&d);
    let out = do_export(&d, Some(&key));
    let other = d.0.join("other.key");
    std::fs::write(&other, "cd".repeat(32)).unwrap();
    let v = verify(&out, Some(&other));
    assert!(!v.ok, "wrong key ⇒ not ok");
    assert!(
        v.signature.starts_with("invalid"),
        "signature: {}",
        v.signature
    );
    assert_eq!(artifact::classify(&v), ValidationState::SignatureInvalid);
}

#[test]
fn extra_files_are_reported_as_extras_not_tampering() {
    let d = tmp("extras");
    let out = do_export(&d, None);
    std::fs::write(out.join("stray.txt"), b"who let the bytes in").unwrap();
    let v = verify(&out, None);
    assert!(
        v.extras.contains(&"stray.txt".to_string()),
        "extras: {:?}",
        v.extras
    );
    // documented semantic: extras never flip per-file promises; classification
    // stays "verified" and the operator sees the list — surfaced, not hidden.
    assert_eq!(
        artifact::classify(&v),
        ValidationState::VerifiedUnsigned,
        "extras are the manifest's siblings, not its violations"
    );
}

#[test]
fn deterministic_exports_and_resume_keep_promises() {
    let d = tmp("deterministic");
    let out1 = do_export(&d, None);
    let m1 = artifact::load(&out1).unwrap();
    // second export into a new dir from the same source
    let out2 = d.0.join("deliver2");
    export::run(&ExportOpts {
        inputs: vec![sample_jsonl(&d)],
        out_dir: out2.clone(),
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    })
    .unwrap();
    let m2 = artifact::load(&out2).unwrap();
    let strip_ts = |m: &artifact::Artifact| {
        let mut c = m.clone();
        c.identity.generated_ts = 0;
        c
    };
    assert_eq!(
        strip_ts(&m1),
        strip_ts(&m2),
        "same input ⇒ identical artifact except the documented generated_ts tick"
    );
    // resume with nothing missing: re-run with resume=true must succeed and
    // keep per-file sha promises (validated parts reused, never trusted blind)
    let rep = export::run(&ExportOpts {
        inputs: vec![sample_jsonl(&d)],
        out_dir: out2.clone(),
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: true,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    })
    .unwrap();
    assert_eq!(rep.manifest.records, 3);
    assert_eq!(
        artifact::classify(&verify(&out2, None)),
        ValidationState::VerifiedUnsigned,
        "re-export keeps the payload byte-identical"
    );
}

#[test]
fn path_guard_refuses_traversal_before_the_filesystem() {
    let p = Path::new("..");
    let err = artifact::safe_manifest_path(&p.join("..")).unwrap_err();
    assert!(err.to_string().contains(".."));
}
