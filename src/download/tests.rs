// Unit corpus for src/download.rs — path safety, state gating, size caps and
// header construction. Every test writes its own files under a temp dir.

use super::*;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-dl-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

fn write(root: &Path, name: &str, body: &str) -> PathBuf {
    let p = root.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn store(root: &Path) -> ArtifactStore {
    ArtifactStore::new(root.to_path_buf())
}

// ---------------------------------------------------------------- file naming

#[test]
fn content_types_cover_the_delivery_formats() {
    assert!(content_type_for("records.csv").starts_with("text/csv"));
    assert!(content_type_for("records.jsonl").contains("ndjson"));
    assert!(content_type_for("manifest.json") == "application/json");
    assert!(content_type_for("r.xlsx").contains("spreadsheetml"));
    assert!(content_type_for("r.pdf") == "application/pdf");
    assert!(content_type_for("r.parquet").contains("parquet"));
    assert!(content_type_for("r.xml") == "application/xml");
    assert!(content_type_for("bundle.zip") == "application/zip");
    // unknown extensions never become something a browser will render
    assert!(content_type_for("payload.exe") == "application/octet-stream");
    assert!(content_type_for("noext") == "application/octet-stream");
}

#[test]
fn safe_filename_strips_everything_dangerous() {
    assert_eq!(safe_filename("records.csv"), "records.csv");
    assert_eq!(safe_filename("../../etc/passwd"), "....etcpasswd");
    assert_eq!(safe_filename("a\"b\\c.csv"), "abc.csv");
    assert_eq!(safe_filename("line\r\nbreak.csv"), "linebreak.csv");
    assert_eq!(safe_filename("   "), "artifact.bin");
    assert_eq!(safe_filename(""), "artifact.bin");
}

#[test]
fn content_disposition_is_always_an_attachment() {
    let d = content_disposition("records.csv");
    assert!(d.starts_with("attachment; filename=\""));
    assert!(d.ends_with('"'));
    assert!(!d.contains('\n'));
    assert!(!d.contains('\r'));
}

// ------------------------------------------------------------------- registry

#[test]
fn register_finalize_and_download_a_real_file() {
    let root = tmp("basic");
    let s = store(&root);
    let p = write(&root, "records.csv", "a,b\n1,2\n");
    let id = s
        .register(
            p,
            "records.csv".to_string(),
            ArtifactRole::Payload,
            Some("job-1".to_string()),
            VerificationState::NotRequested,
        )
        .unwrap();
    assert_eq!(s.get(id.as_str()).unwrap().state, ArtifactState::Writing);
    // not downloadable until finalized
    assert!(s.open(id.as_str()).is_err());
    let body = std::fs::read(root.join("records.csv")).unwrap();
    s.finalize(id.as_str(), crate::control::sha256_hex(&body))
        .unwrap();

    let rec = s.get(id.as_str()).unwrap();
    assert_eq!(rec.state, ArtifactState::Ready);
    assert_eq!(rec.bytes, 8);
    assert_eq!(rec.role, ArtifactRole::Payload);
    assert!(rec.to_json()["downloadable"].as_bool().unwrap());

    let opened = s.open(id.as_str()).unwrap();
    assert_eq!(opened.read().unwrap(), b"a,b\n1,2\n");
    assert!(opened.content_type().starts_with("text/csv"));
    assert_eq!(
        opened.content_disposition(),
        "attachment; filename=\"records.csv\""
    );
}

#[test]
fn registered_metadata_never_exposes_the_path() {
    let root = tmp("nopath");
    let s = store(&root);
    let p = write(&root, "manifest.json", "{}");
    let id = s
        .register(
            p,
            "manifest.json".to_string(),
            ArtifactRole::Manifest,
            None,
            VerificationState::NotRequested,
        )
        .unwrap();
    let json = s.get(id.as_str()).unwrap().to_json();
    let text = serde_json::to_string(&json).unwrap();
    assert!(!text.contains("path"), "no path key at all: {text}");
    assert!(!text.contains(&root.to_string_lossy().to_string()));
    // the struct itself keeps the path server-side
    assert!(std::fs::metadata(root.join("manifest.json")).is_ok());
}

#[test]
fn a_file_outside_the_root_cannot_be_registered() {
    let root = tmp("outside");
    let elsewhere = tmp("outside-other");
    let s = store(&root);
    let p = write(&elsewhere, "secret.csv", "x");
    let e = s
        .register(
            p,
            "secret.csv".to_string(),
            ArtifactRole::Payload,
            None,
            VerificationState::NotRequested,
        )
        .unwrap_err();
    assert_eq!(e.status(), 409);
    assert!(e.message.contains("outside the control root"));
}

#[test]
fn unknown_and_malformed_ids_are_not_found() {
    let root = tmp("ids");
    let s = store(&root);
    assert_eq!(s.open("art-999").unwrap_err().status(), 404);
    // even a syntactically valid id that was never minted is a 404
    assert_eq!(s.open("../../etc/passwd").unwrap_err().status(), 404);
    assert_eq!(s.open("").unwrap_err().status(), 404);
}

#[test]
fn a_writing_artifact_is_refused_with_a_conflict() {
    let root = tmp("writing");
    let s = store(&root);
    let p = write(&root, "part.csv", "a");
    let id = s
        .register(
            p,
            "part.csv".to_string(),
            ArtifactRole::Payload,
            Some("job-2".to_string()),
            VerificationState::NotRequested,
        )
        .unwrap();
    let e = s.open(id.as_str()).unwrap_err();
    assert_eq!(e.status(), 409);
    assert!(e.message.contains("still being written"));
}

#[test]
fn failed_verification_disables_the_download() {
    let root = tmp("verifyfail");
    let s = store(&root);
    let p = write(&root, "records.csv", "a,b\n");
    let id = s
        .register(
            p.clone(),
            "records.csv".to_string(),
            ArtifactRole::Payload,
            Some("job-3".to_string()),
            VerificationState::NotRequested,
        )
        .unwrap();
    s.finalize(id.as_str(), "deadbeef".to_string()).unwrap();
    s.mark_verified(
        id.as_str(),
        VerificationState::Failed {
            reason: "1 file(s) failed their manifest check".to_string(),
        },
    );
    let rec = s.get(id.as_str()).unwrap();
    assert_eq!(rec.state, ArtifactState::Failed);
    assert_eq!(rec.verification.as_str(), "failed");
    let e = s.open(id.as_str()).unwrap_err();
    assert_eq!(e.status(), 409);
    assert!(e.message.contains("manifest check"), "{e:?}");
}

#[test]
fn successful_verification_is_carried_on_the_record() {
    let root = tmp("verifyok");
    let s = store(&root);
    let p = write(&root, "records.csv", "a,b\n");
    let id = s
        .register(
            p,
            "records.csv".to_string(),
            ArtifactRole::Payload,
            Some("job-4".to_string()),
            VerificationState::NotRequested,
        )
        .unwrap();
    s.finalize(id.as_str(), "abc".to_string()).unwrap();
    s.mark_verified(
        id.as_str(),
        VerificationState::Ok {
            records: 1,
            records_basis: "jsonl-lines".to_string(),
            files: 3,
        },
    );
    let json = s.get(id.as_str()).unwrap().to_json();
    assert_eq!(json["verification"]["state"], "ok");
    assert_eq!(json["verification"]["records"], 1);
    assert_eq!(json["verification"]["records_basis"], "jsonl-lines");
}

#[test]
fn oversized_artifacts_are_refused_before_and_after_reading() {
    let root = tmp("toobig");
    let s = store(&root);
    let p = write(&root, "big.csv", "0123456789");
    let id = s
        .register(
            p,
            "big.csv".to_string(),
            ArtifactRole::Payload,
            None,
            VerificationState::NotRequested,
        )
        .unwrap();
    s.finalize(id.as_str(), "0".repeat(64)).unwrap();
    assert!(
        s.open(id.as_str()).is_ok(),
        "10 bytes is under the default cap"
    );
    let e = s.open_bounded(id.as_str(), 4).unwrap_err();
    assert_eq!(e.status(), 413);
    assert!(e.message.contains("over the 4 byte"));
}

#[test]
fn a_file_that_vanishes_fails_closed() {
    let root = tmp("gone");
    let s = store(&root);
    let p = write(&root, "gone.csv", "x");
    let id = s
        .register(
            p.clone(),
            "gone.csv".to_string(),
            ArtifactRole::Payload,
            None,
            VerificationState::NotRequested,
        )
        .unwrap();
    s.finalize(id.as_str(), "abc".to_string()).unwrap();
    std::fs::remove_file(&p).unwrap();
    let e = s.open(id.as_str()).unwrap_err();
    // not a 404: the artifact exists, the file does not (or moved), and
    // neither case may end in "serve something else instead"
    assert_eq!(e.status(), 409);
}

#[test]
fn failing_a_job_fails_only_its_unfinished_artifacts() {
    let root = tmp("failjob");
    let s = store(&root);
    let a = s
        .register(
            write(&root, "a.csv", "a"),
            "a.csv".to_string(),
            ArtifactRole::Payload,
            Some("job-9".to_string()),
            VerificationState::NotRequested,
        )
        .unwrap();
    let b = s
        .register(
            write(&root, "b.csv", "b"),
            "b.csv".to_string(),
            ArtifactRole::Payload,
            Some("job-9".to_string()),
            VerificationState::NotRequested,
        )
        .unwrap();
    s.finalize(a.as_str(), "x".to_string()).unwrap();
    s.fail_job_artifacts("job-9", "the job was cancelled");
    // a was already finalized, so it stays downloadable
    assert_eq!(s.get(a.as_str()).unwrap().state, ArtifactState::Ready);
    // b was still writing, so it is failed with a reason
    let rec = s.get(b.as_str()).unwrap();
    assert_eq!(rec.state, ArtifactState::Failed);
    assert_eq!(rec.failure.as_deref(), Some("the job was cancelled"));
}

#[test]
fn for_job_lists_a_jobs_artifacts() {
    let root = tmp("forjob");
    let s = store(&root);
    for (i, job) in ["job-1", "job-2"].iter().enumerate() {
        s.register(
            write(&root, &format!("f{i}.csv"), "x"),
            format!("f{i}.csv"),
            ArtifactRole::Payload,
            Some(job.to_string()),
            VerificationState::NotRequested,
        )
        .unwrap();
    }
    assert_eq!(s.for_job("job-1").len(), 1);
    assert_eq!(s.list().len(), 2);
    assert_eq!(s.for_job("job-1")[0].name, "f0.csv");
}

#[test]
fn verification_state_json_shapes_are_stable() {
    assert_eq!(
        VerificationState::NotRequested.to_json()["state"],
        "not_requested"
    );
    let ok = VerificationState::Ok {
        records: 3,
        records_basis: "manifest-declared".to_string(),
        files: 2,
    };
    assert_eq!(ok.to_json()["state"], "ok");
    assert_eq!(ok.to_json()["files"], 2);
    let bad = VerificationState::Failed {
        reason: "nope".to_string(),
    };
    assert_eq!(bad.to_json()["reason"], "nope");
    assert_eq!(bad.as_str(), "failed");
}
