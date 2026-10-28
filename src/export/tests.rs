use super::*;

use serde_json::json;

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-export-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fixture(dir: &Path) -> PathBuf {
    let lines: Vec<String> = vec![
        json!({"run":"r1","target":"catalog","url":"http://x/1","depth":0,"ts":10,"fingerprint":"a1","egress":"direct",
               "fields":{"title":"One, two","price":"$5","tags":["t1","t2"],"meta":{"stock":true}}}).to_string(),
        json!({"run":"r1","target":"catalog","url":"http://x/2","depth":1,"ts":11,"fingerprint":"a2","egress":"direct",
               "fields":{"title":"say \"hi\"","price":"$7","meta":{"stock":false,"extra":{"k":1}}}}).to_string(),
        json!({"run":"r1","target":"api","url":"http://x/j","depth":0,"ts":12,"fingerprint":"a3","egress":"proxy-1",
               "fields":{"count":2,"ok":true,"ids":[3,4]}}).to_string(),
        "not json at all".to_string(),
    ];
    let p = dir.join("in.jsonl");
    std::fs::write(
        &p,
        lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
    )
    .unwrap();
    p
}

#[cfg(unix)]
#[test]
fn export_rejects_symlinked_inputs_and_history_files() {
    use std::os::unix::fs::symlink;

    let dir = workdir("no-follow-inputs");
    let victim = dir.join("private.jsonl");
    let input = dir.join("input-link.jsonl");
    std::fs::write(
        &victim,
        format!("{}\n", json!({"url":"https://example.test/", "fields":{}})),
    )
    .unwrap();
    symlink(&victim, &input).unwrap();
    let options = ExportOpts {
        inputs: vec![input.clone()],
        out_dir: dir.join("delivery"),
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    assert!(
        run(&options).is_err(),
        "a symlink input must not be followed"
    );

    let history = dir.join("history-link.jsonl");
    symlink(&victim, &history).unwrap();
    assert!(policy_for_runs(&history, &BTreeSet::new()).is_err());
    assert!(history_counters(&history, &[]).is_none());
}

#[cfg(unix)]
#[test]
fn progress_publish_replaces_hardlink_without_truncating_its_target() {
    use std::fs::hard_link;

    let dir = workdir("progress-hardlink");
    let victim = dir.join("victim");
    std::fs::write(&victim, b"operator-owned bytes").unwrap();
    hard_link(&victim, dir.join(PROGRESS_FILE)).unwrap();

    let progress = Progress {
        fingerprint: "test-fingerprint".to_string(),
        files: Vec::new(),
    };
    save_progress(&dir, &progress).unwrap();
    assert_eq!(std::fs::read(&victim).unwrap(), b"operator-owned bytes");
    assert_eq!(load_progress(&dir).unwrap().fingerprint, "test-fingerprint");
}

#[test]
fn field_flattening_and_column_union_are_exact() {
    // The CSV contract operators type against: nested objects become
    // dot-paths, arrays stay atomic, and the column union is
    // meta-cols-first + sorted (diff-stable output).
    let v = json!({
        "url": "http://x/1", "fields": {
            "a": { "b": { "c": 1 } },
            "z": [2, 3],
            "n": null
        }
    });
    let (_meta, payload) = split_row(&v);
    assert_eq!(payload["a.b.c"], json!(1));
    assert_eq!(payload["z"], json!([2, 3]));
    assert_eq!(payload["n"], json!(null));

    let v2 = json!({ "url": "u2", "fields": { "a": 1, "m": "x" } });
    let r2 = split_row(&v2);
    let cols = target_columns(&[&r1(&v), &r2]);
    assert_eq!(cols.first().unwrap().as_str(), META_COLS[0]);
    let tail: Vec<&str> = cols[META_COLS.len()..].iter().map(String::as_str).collect();
    // scalar `a` (row 2) and object-expanded `a.b.c` (row 1) coexist —
    // a real schema change mid-export, faithfully unioned.
    assert_eq!(tail, vec!["a", "a.b.c", "m", "n", "z"]);
    assert_eq!(cols.len(), META_COLS.len() + 5, "no duplicate columns");
}

fn r1(v: &Value) -> (Vec<Value>, Map<String, Value>) {
    split_row(v)
}

#[test]
fn csv_escape_covers_cr_and_quotes() {
    // CR must also quote (Excel's line ending); quotes double per RFC4180.
    let cr = csv_escape(&format!("a{}b", '\r'));
    assert!(cr.starts_with('"') && cr.ends_with('"') && cr.contains(&format!("a{}b", '\r')));
    let t = csv_escape("he said \"hi\", ok");
    assert_eq!(t, "\"he said \"\"hi\"\", ok\"");
    assert_eq!(csv_escape("plain"), "plain");
    assert_eq!(cell(&json!(null)), "");
    assert_eq!(cell(&json!(true)), "true");
}

#[test]
fn deterministic_csv_jsonl_and_manifest() {
    let dir = workdir("det");
    let input = fixture(&dir);
    let o1 = ExportOpts {
        inputs: vec![input.clone()],
        out_dir: dir.join("a"),
        split_target: false,
        history: None,
        key: None,

        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    let r1 = run(&o1).unwrap();
    let o2 = ExportOpts {
        inputs: vec![input],
        out_dir: dir.join("b"),
        split_target: false,
        history: None,
        key: None,

        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    let r2 = run(&o2).unwrap();

    // byte-identical payloads across runs; manifests differ only in generated_ts
    let csv1 = std::fs::read_to_string(dir.join("a/records.csv")).unwrap();
    let csv2 = std::fs::read_to_string(dir.join("b/records.csv")).unwrap();
    assert_eq!(csv1, csv2, "CSV export must be deterministic");
    let strip = |p: &Path| -> String {
        std::fs::read_to_string(p)
            .unwrap()
            .lines()
            .filter(|l| !l.contains("generated_ts"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        strip(&dir.join("a/manifest.json")),
        strip(&dir.join("b/manifest.json"))
    );
    assert_eq!(r1.manifest.records, r2.manifest.records);

    // columns: meta first, then the sorted union of all targets' keys
    let header_line = csv1.lines().next().unwrap();
    assert_eq!(
        header_line,
        "run,target,url,depth,ts,fingerprint,egress,count,ids,meta.extra.k,meta.stock,ok,price,tags,title"
    );

    // quoting: embedded comma + doubled quotes, flattened dotted cells, array as compact JSON
    assert!(csv1.contains("\"One, two\""), "comma cell must be quoted");
    assert!(
        csv1.contains("\"say \"\"hi\"\"\""),
        "quotes must double-escape"
    );
    // RFC-4180 escaping: array cell is compact JSON, and its quotes are
    // doubled inside the surrounding quoted field.
    assert!(
        csv1.contains("\"[\"\"t1\"\",\"\"t2\"\"]\""),
        "array cell = escaped compact JSON: {csv1}"
    );
    assert_eq!(csv1.lines().count(), 4, "header + 3 rows");

    // manifest math + checksums actually verify
    let m = &r1.manifest;
    assert_eq!(m.records, 3);
    assert_eq!(m.skipped_lines, 1);
    assert_eq!(m.unique_urls, 3);
    assert_eq!(m.targets["catalog"], 2);
    assert_eq!(m.targets["api"], 1);
    for fe in &m.files {
        let bytes = std::fs::read(dir.join("a").join(&fe.name)).unwrap();
        let mut h = Sha256::new();
        h.update(&bytes);
        let sha = h
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(sha, fe.sha256, "manifest sha must match file bytes");
        assert_eq!(bytes.len() as u64, fe.bytes);
    }

    // jsonl canonical form: flattened field names are now real keys under fields
    let j: Value = serde_json::from_str(
        std::fs::read_to_string(dir.join("a/records.jsonl"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(j["fields"]["meta.stock"], json!(true));
    assert_eq!(j["fields"]["tags"], json!(["t1", "t2"]));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn split_target_and_history_policy_link() {
    let dir = workdir("split");
    let input = fixture(&dir);
    let hist = dir.join("runs.jsonl");
    std::fs::write(
        &hist,
        format!(
            "{}\n{}\n",
            json!({"run":"r1","ts":1,"config_sha256":"old"}),
            json!({"run":"r1","ts":2,"config_sha256":"latest"})
        ),
    )
    .unwrap();
    let o = ExportOpts {
        inputs: vec![input],
        out_dir: dir.join("out"),
        split_target: true,
        history: Some(hist),
        key: None,

        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    let r = run(&o).unwrap();
    let names: Vec<&str> = r.manifest.files.iter().map(|f| f.name.as_str()).collect();
    assert!(
        names.contains(&"catalog.csv") && names.contains(&"api.jsonl"),
        "{names:?}"
    );
    // per-target columns: api file must not carry catalog-only columns
    let api = std::fs::read_to_string(dir.join("out/api.csv")).unwrap();
    assert!(!api.contains("title"), "api columns stay scoped: {api}");
    assert_eq!(
        r.manifest.policy_sha256.as_ref().unwrap()["r1"],
        "latest",
        "newest history row wins"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_and_garbage_inputs_fail_loudly() {
    let dir = workdir("bad");
    let p = dir.join("in.jsonl");
    std::fs::write(&p, "\"just a string\"\n[1,2]\n\n").unwrap();
    let o = ExportOpts {
        inputs: vec![p],
        out_dir: dir.join("out"),
        split_target: false,
        history: None,
        key: None,

        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    let e = run(&o).unwrap_err().to_string();
    assert!(e.contains("no valid records"), "{e}");
    // missing input file is a clear error too
    let o2 = ExportOpts {
        inputs: vec![dir.join("nope.jsonl")],
        out_dir: dir.join("out"),
        split_target: false,
        history: None,
        key: None,

        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    assert!(run(&o2).unwrap_err().to_string().contains("read"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn manifest_signature_attests_the_folder_and_detects_edits() {
    let dir = workdir("signtest");
    let input = fixture(&dir);
    let key = dir.join("attest.hex");
    crate::sign::Key::generate(&key).unwrap();
    let o = ExportOpts {
        inputs: vec![input],
        out_dir: dir.join("out"),
        split_target: false,
        history: None,
        key: Some(key.clone()),

        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    let r = run(&o).unwrap();
    let sig = r.manifest_sig.expect("manifest_sig present when key given");
    assert!(sig.ends_with("manifest.json.sig"), "{sig}");
    let manifest = dir.join("out/manifest.json");
    crate::sign::Key::load(&key)
        .unwrap()
        .verify_file(&manifest, std::path::Path::new(&sig))
        .expect("fresh delivery verifies");

    // One edited byte inside the manifest (e.g. someone "fixing" a count)
    // must break verification.
    let mut raw = std::fs::read(&manifest).unwrap();
    raw[10] ^= 0x01; // flip one bit, wherever it lands
    std::fs::write(&manifest, &raw).unwrap();
    let e = crate::sign::Key::load(&key)
        .unwrap()
        .verify_file(&manifest, std::path::Path::new(&sig))
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("HMAC verification FAILED"),
        "{e:#}"
    );

    // No key => no signature file at all (opt-in stays zero-cost).
    let o2 = ExportOpts {
        inputs: vec![dir.join("nope.jsonl")],
        out_dir: dir.join("out2"),
        split_target: false,
        history: None,
        key: None,

        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    assert!(run(&o2).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn zip_writer_emits_a_readable_archive() {
    let dir = std::path::PathBuf::from("/tmp/harvest-zip-check");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "hello bundle\n").unwrap();
    std::fs::write(dir.join("b.txt"), "second member\n").unwrap();
    let out = dir.join("bundle.zip");
    let files = vec![
        ("a.txt".to_string(), dir.join("a.txt")),
        ("b.txt".to_string(), dir.join("b.txt")),
    ];
    let report = super::bundle::write_zip_from_files(&out, &files).unwrap();
    let raw = std::fs::read(&out).unwrap();
    assert_eq!(&raw[..4], b"PK\x03\x04");
    assert!(raw.windows(4).rev().any(|w| w == b"PK\x05\x06"));
    assert_eq!(report.entries.len(), 2);
    assert_eq!(
        report.entries[0].crc,
        super::bundle::crc32(b"hello bundle\n")
    );
    // The report's digest is the digest of the archive file itself — the value
    // a client re-hashes after a download.
    let mut h = sha2::Sha256::new();
    h.update(&raw);
    assert_eq!(report.sha256, format!("{:x}", h.finalize()));
    assert_eq!(report.bytes, raw.len() as u64);
}

#[test]
fn zip_writer_refuses_unsafe_member_names() {
    let dir = std::path::PathBuf::from("/tmp/harvest-zip-check2");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    for bad in [
        "../escape.txt",
        "/abs.txt",
        "a\\b.txt",
        "",
        "dir/../../x.txt",
    ] {
        let files = vec![(bad.to_string(), dir.join("a.txt"))];
        let err = super::bundle::write_zip_from_files(&dir.join("bad.zip"), &files).unwrap_err();
        assert!(err.to_string().contains("archive member name"), "{}", err);
    }
}

#[test]
fn audit_empty_manifest_must_not_verify_successfully() {
    let dir = std::env::temp_dir().join(format!("harvest-audit-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let m = serde_json::json!({"v":1,"generated_ts":1,"source_files":[],"records":1,"unique_urls":1,"skipped_lines":0,"runs":[],"targets":{},"policy_sha256":null,"split_target":false,"columns":[],"files":[]});
    std::fs::write(dir.join("manifest.json"), m.to_string()).unwrap();
    assert!(
        !verify(&dir, None).unwrap().ok,
        "no payload cannot be a verified delivery"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn audit_manifest_path_traversal_is_refused() {
    let base = std::env::temp_dir().join(format!("harvest-audit-traversal-{}", std::process::id()));
    let dir = base.join("delivery");
    std::fs::create_dir_all(&dir).unwrap();
    let body = b"private\n";
    std::fs::write(base.join("private.txt"), body).unwrap();
    let m = serde_json::json!({"v":1,"generated_ts":1,"source_files":[],"records":1,"unique_urls":1,"skipped_lines":0,"runs":[],"targets":{},"policy_sha256":null,"split_target":false,"columns":[],"files":[{"name":"../private.txt","sha256":digest_hex(Sha256::digest(body)),"bytes":body.len(),"lines":1}]});
    std::fs::write(dir.join("manifest.json"), m.to_string()).unwrap();
    assert!(
        !verify(&dir, None).unwrap().ok,
        "must not verify an external file"
    );
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn audit_flattening_preserves_literal_paths_empty_objects_and_metadata_names() {
    let value = json!({"run":"meta-run","fields":{"a.b":1,"a":{"b":2},"empty":{},"run":"payload-run","fields:run":"literal-key"}});
    let row = split_row(&value);
    assert_eq!(row.1["a\\.b"], 1);
    assert_eq!(row.1["a.b"], 2);
    assert_eq!(row.1["empty"], json!({}));
    assert_eq!(row.1["fields:run"], "payload-run");
    assert_eq!(row.1["fields\\:run"], "literal-key");
    let table = records_json(&[&row], &target_columns(&[&row]));
    assert_eq!(table[0]["run"], "meta-run");
    assert_eq!(table[0]["fields:run"], "payload-run");
}

#[test]
fn audit_forged_resume_sidecar_cannot_substitute_its_own_payload() {
    let dir = workdir("audit-forged-resume");
    let input = fixture(&dir);
    let mut options = ExportOpts {
        inputs: vec![input],
        out_dir: dir.join("delivery"),
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: vec![ExportFormat::Jsonl],
        dataset: None,
        pdf_sample_rows: 12,
    };
    run(&options).unwrap();
    let expected = std::fs::read(options.out_dir.join("records.jsonl")).unwrap();
    let wrong = b"{\"run\":\"forged\",\"fields\":{\"title\":\"wrong\"}}\n";
    std::fs::write(options.out_dir.join("records.jsonl"), wrong).unwrap();
    options.resume = true;
    let entry = file_entry(&options.out_dir, "records.jsonl").unwrap();
    save_progress(
        &options.out_dir,
        &Progress {
            fingerprint: run_fingerprint(&options).unwrap(),
            files: vec![entry],
        },
    )
    .unwrap();
    run(&options).unwrap();
    assert_eq!(
        std::fs::read(options.out_dir.join("records.jsonl")).unwrap(),
        expected
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn audit_checksum_only_folder_cannot_claim_to_deliver_records() {
    let dir = workdir("audit-checksum-only");
    std::fs::write(dir.join("checksums.txt"), b"").unwrap();
    let value = json!({"v":1,"generated_ts":1,"source_files":[],"split_target":false,"records":100,"unique_urls":0,"skipped_lines":0,"runs":[],"targets":{},"columns":[],"files":[{"name":"checksums.txt","sha256":digest_hex(Sha256::digest(b"")),"bytes":0,"lines":0}]});
    std::fs::write(dir.join("manifest.json"), value.to_string()).unwrap();
    assert!(!verify(&dir, None).unwrap().ok);
    std::fs::remove_dir_all(dir).ok();
}
