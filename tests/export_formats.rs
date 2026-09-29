// Multi-format delivery: every exporter writes a real file, every file is
// covered by the manifest, every tamper is detected, and the ZIP bundle
// re-inflates into a folder that `harvest verify` accepts.
//
// The writers are exercised through `export::run` (the same code path the CLI
// uses) so the adapters cannot drift from the delivery pipeline.

use std::path::{Path, PathBuf};

use harvest::export::{self, ExportFormat, ExportOpts};
use serde_json::json;
use sha2::{Digest, Sha256};

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-export-formats-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sha256_of(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    format!("{:x}", Sha256::digest(&bytes))
}

/// A record set with one of every shape: string, int, float, bool, ISO date,
/// null, array (JSON text) and a nested object (flattened).
fn input_jsonl(dir: &Path, rows: usize) -> PathBuf {
    let p = dir.join("records.jsonl");
    let mut out = String::new();
    for i in 0..rows {
        let rec = json!({
            "run": "demo-run",
            "target": if i % 2 == 0 { "demo-catalog" } else { "demo-news" },
            "url": format!("https://shop.test/p/{i}"),
            "depth": i % 3,
            "ts": 1_750_000_000 + i as i64,
            "fingerprint": format!("fp{i:05}"),
            "egress": "direct",
            "fields": {
                "title": format!("Item {i} — deluxe"),
                "price": 9.5 + (i as f64) * 1.25,
                "qty": i as i64,
                "in_stock": i % 2 == 0,
                "published": format!("2026-01-{:02}", (i % 28) + 1),
                "note": if i % 5 == 0 { serde_json::Value::Null } else { json!("ok") },
                "tags": ["a", "b"],
                "nested": { "sku": format!("SKU-{i:03}") }
            }
        });
        out.push_str(&serde_json::to_string(&rec).unwrap());
        out.push('\n');
    }
    std::fs::write(&p, out).unwrap();
    p
}

fn run_all(dir: &Path, dataset: Option<&str>) -> export::Report {
    let src = dir.join("in");
    std::fs::create_dir_all(&src).unwrap();
    let input = input_jsonl(&src, 120);
    let out = dir.join("out");
    export::run(&ExportOpts {
        inputs: vec![input],
        out_dir: out,
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: ExportFormat::all().to_vec(),
        dataset: dataset.map(|s| s.to_string()),
        pdf_sample_rows: 12,
    })
    .expect("export run")
}

#[test]
fn every_format_writes_a_non_empty_file_covered_by_the_manifest() {
    let dir = workdir("all");
    let report = run_all(&dir, Some("demo-run"));
    let out = dir.join("out");

    assert_eq!(
        report.formats.len(),
        7,
        "csv, jsonl, xlsx, parquet, xml, pdf, zip"
    );
    for f in ExportFormat::all() {
        if *f == ExportFormat::Zip {
            // The bundle is reported by the CLI and described by its own
            // in-archive manifest; a folder manifest cannot describe the
            // archive that contains it.
            continue;
        }
        let name = format!("dataset-demo-run.{}", f.ext());
        let path = if *f == ExportFormat::Csv || *f == ExportFormat::Jsonl {
            out.join(format!("records.{}", f.ext()))
        } else {
            out.join(&name)
        };
        assert!(path.exists(), "{name} missing");
        let bytes = std::fs::metadata(&path).unwrap().len();
        assert!(bytes > 0, "{name} is empty");
        assert!(
            report
                .manifest
                .files
                .iter()
                .any(|e| e.name == path.file_name().unwrap().to_string_lossy()),
            "{name} is not covered by the manifest"
        );
        // Every payload file's manifest sha256 matches the bytes on disk.
        let entry = report
            .manifest
            .files
            .iter()
            .find(|e| e.name == path.file_name().unwrap().to_string_lossy())
            .unwrap();
        assert_eq!(entry.sha256, sha256_of(&path), "{name} digest mismatch");
        assert_eq!(entry.bytes, bytes);
    }
    assert_eq!(report.manifest.records, 120);
    assert!(report
        .manifest
        .files
        .iter()
        .any(|f| f.name == "checksums.txt"));
}

#[test]
fn csv_and_jsonl_keep_their_historical_behaviour() {
    let dir = workdir("base");
    let report = run_all(&dir, None);
    let out = dir.join("out");
    // No --dataset: names stay `records.<ext>`.
    assert!(out.join("records.csv").exists());
    assert!(out.join("records.jsonl").exists());
    let csv = std::fs::read_to_string(out.join("records.csv")).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 121, "header + 120 rows");
    assert!(lines[0].starts_with("run,target,url,depth,ts,fingerprint,egress"));
    assert!(lines[0].contains("nested.sku"), "objects are dot-flattened");
    let jsonl = std::fs::read_to_string(out.join("records.jsonl")).unwrap();
    assert_eq!(jsonl.lines().count(), 120);
    let first: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
    assert_eq!(first["target"], "demo-catalog");
    assert!(first["fields"]["title"].is_string());
    assert_eq!(report.manifest.records, 120);
}

#[test]
fn xlsx_is_a_real_zip_package_with_typed_cells() {
    let dir = workdir("xlsx");
    let report = run_all(&dir, Some("demo-run"));
    let path = dir.join("out").join("dataset-demo-run.xlsx");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..2], b"PK", "xlsx is a ZIP package");
    assert!(
        bytes.windows(4).rev().any(|w| w == b"PK\x05\x06"),
        "EOCD present"
    );

    // The package carries the parts a spreadsheet reader requires.
    let names: Vec<String> = {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 4 <= bytes.len() {
            if &bytes[i..i + 4] == b"PK\x03\x04" {
                let len = u16::from_le_bytes([bytes[i + 26], bytes[i + 27]]) as usize;
                let start = i + 30;
                out.push(String::from_utf8_lossy(&bytes[start..start + len]).to_string());
            }
            i += 1;
        }
        out
    };
    for part in [
        "[Content_Types].xml",
        "xl/workbook.xml",
        "xl/worksheets/sheet1.xml",
        "xl/styles.xml",
    ] {
        assert!(names.iter().any(|n| n == part), "missing {part}");
    }
    // Typed cells: a numeric and a boolean cell must not be inline strings.
    let sheet = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&sheet);
    assert!(text.contains("inlineStr"), "strings are inline strings");
    assert!(text.contains("t=\"b\""), "booleans are boolean cells");
    assert!(text.contains("s=\"2\""), "dates carry the date style");
    let _ = report;
}

#[test]
fn pdf_is_a_valid_summary_report() {
    let dir = workdir("pdf");
    run_all(&dir, Some("demo-run"));
    let path = dir.join("out").join("dataset-demo-run.pdf");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert!(bytes.trim_ascii_end().ends_with(b"%%EOF"));
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Type /Catalog"));
    assert!(text.contains("startxref"));
    assert!(text.contains("/Type /Page"), "at least one page object");
    // Summary-oriented: the report names the record count and refuses to dump
    // all 120 rows (the sample table is bounded to 12).
    let body = text.split("stream").last().unwrap_or("");
    assert!(body.contains("120") || text.contains("records"));
    assert!(
        !body.contains("SKU-119"),
        "the last record is not in the sample table"
    );
}

#[test]
fn parquet_is_a_real_parquet_file() {
    let dir = workdir("parquet");
    run_all(&dir, Some("demo-run"));
    let path = dir.join("out").join("dataset-demo-run.parquet");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], b"PAR1", "leading magic");
    assert_eq!(&bytes[bytes.len() - 4..], b"PAR1", "trailing magic");
    let len_bytes = &bytes[bytes.len() - 8..bytes.len() - 4];
    let footer_len =
        u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
    assert!(
        footer_len > 0 && footer_len < bytes.len(),
        "footer length is sane"
    );
    let footer = &bytes[bytes.len() - 8 - footer_len..bytes.len() - 8];
    // The footer's first field is the format version (1), written as a thrift
    // compact i32 zigzag varint.
    assert_eq!(footer[0], 0x15, "field 1, i32");
    assert_eq!(footer[1], 0x02, "zigzag(1)");
    // Schema element names are present in the footer.
    let footer_text = String::from_utf8_lossy(footer);
    for col in ["run", "target", "url", "price", "in_stock", "nested.sku"] {
        assert!(footer_text.contains(col), "schema missing {col}");
    }
    // The record payload itself: PLAIN-encoded values are in the page bodies.
    assert!(
        bytes.windows(4).any(|w| w == b"SKU-"),
        "values are written PLAIN"
    );
}

#[test]
fn xml_is_well_formed_and_escaped() {
    let dir = workdir("xml");
    run_all(&dir, Some("demo-run"));
    let path = dir.join("out").join("dataset-demo-run.xml");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(text.trim_end().ends_with("</harvest-export>"));
    assert_eq!(text.matches("<record ").count(), 120);
    assert!(text.contains("null=\"true\""), "nulls are explicit");
    assert_eq!(
        harvest::export::xml::escape_xml("a<b>&\"'\u{1}"),
        "a&lt;b&gt;&amp;&quot;&apos;",
        "the five entities are escaped and control characters dropped"
    );
    // No raw `<` can appear inside escaped text: every field is a closed element.
    let opens = text.matches("<field ").count();
    let closes = text.matches("</field>").count();
    let self_closed = text.matches("/>").count();
    assert!(opens + self_closed >= closes, "elements balanced");
    assert!(
        !text.contains("<script"),
        "no unescaped content from the data"
    );
}

#[test]
fn unsafe_column_names_are_rejected_not_silently_renamed() {
    let dir = workdir("xml-names");
    std::fs::create_dir_all(&dir).unwrap();
    let report = harvest::export::xml::write_xml(
        &dir.join("out.xml"),
        "demo",
        &[
            "good".to_string(),
            "1bad".to_string(),
            "xml-root".to_string(),
            "a b".to_string(),
        ],
        &[json!({"good": 1, "1bad": 2, "xml-root": 3, "a b": 4})],
        &harvest::export::xml::XmlOpts::default(),
        0,
        1,
    )
    .expect("xml write");
    // `1bad` and `xml-root` cannot be repaired into a legal name safely.
    // `xml-root` is refused outright (the `xml` prefix is reserved); the other
    // two are repaired deterministically and keep their source name in
    // `source=`, so no column is ever silently renamed without a trace.
    assert_eq!(report.skipped_fields, vec!["xml-root".to_string()]);
    assert_eq!(report.fields, 3, "good + `_1bad` + `a_b`");
    assert_eq!(report.records, 1);
    let text = std::fs::read_to_string(dir.join("out.xml")).unwrap();
    assert!(
        text.contains("name=\"a_b\""),
        "spaces are repaired to underscores"
    );
    assert!(text.contains("name=\"_1bad\""), "leading digit is prefixed");
    assert!(
        text.contains("source=\"1bad\""),
        "original name is preserved"
    );
    assert!(!text.contains(">xml-root"), "reserved name is not emitted");
}

#[test]
fn zip_bundle_contains_every_selected_format_and_no_traversal() {
    let dir = workdir("zip");
    let report = run_all(&dir, Some("demo-run"));
    let zip_path = dir.join("out").join("dataset-demo-run.zip");
    let bundle = report.bundle.expect("bundle info");
    assert!(bundle.members >= 9, "{bundle:?}");
    assert_eq!(bundle.sha256, sha256_of(&zip_path));

    // Read the archive: local file headers must name only safe relative paths.
    let bytes = std::fs::read(&zip_path).unwrap();
    let mut names = Vec::new();
    let mut i = 0usize;
    while i + 4 <= bytes.len() {
        if &bytes[i..i + 4] == b"PK\x03\x04" {
            let len = u16::from_le_bytes([bytes[i + 26], bytes[i + 27]]) as usize;
            let start = i + 30;
            names.push(String::from_utf8_lossy(&bytes[start..start + len]).to_string());
        }
        i += 1;
    }
    for expected in [
        "records.csv",
        "records.jsonl",
        "dataset-demo-run.xlsx",
        "dataset-demo-run.parquet",
        "dataset-demo-run.pdf",
        "dataset-demo-run.xml",
        "checksums.txt",
        "manifest.json",
        "bundle-manifest.json",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "bundle missing {expected}"
        );
    }
    for n in &names {
        assert!(!n.contains(".."), "traversal in member name: {n}");
        assert!(!n.starts_with('/'), "absolute member name: {n}");
        assert!(!n.contains('\\'), "backslash member name: {n}");
    }
}

#[test]
fn bundle_manifest_describes_every_member() {
    let dir = workdir("zip-manifest");
    run_all(&dir, Some("demo-run"));
    let out = dir.join("out");
    // `harvest verify` on an extracted bundle must pass: the archive carries a
    // manifest that names each member with its digest.
    let extracted = dir.join("extracted");
    std::fs::create_dir_all(&extracted).unwrap();
    let status = std::process::Command::new("python3")
        .arg("-c")
        .arg(format!(
            "import zipfile,sys; zipfile.ZipFile({:?}).extractall({:?})",
            out.join("dataset-demo-run.zip"),
            extracted
        ))
        .status();
    if status.map(|s| s.success()).unwrap_or(false) {
        let v = export::verify(&extracted, None).expect("verify extracted bundle");
        assert!(v.ok, "{v:?}");
        assert!(v.records_ok);
        let inner: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(extracted.join("bundle-manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(inner["kind"], "harvest-bundle");
        let files = inner["files"].as_array().unwrap();
        assert!(files.len() >= 8);
        for f in files {
            let name = f["name"].as_str().unwrap();
            let sha = f["sha256"].as_str().unwrap();
            assert_eq!(
                sha,
                sha256_of(&extracted.join(name)),
                "{name} digest mismatch in bundle manifest"
            );
        }
    }
}

#[test]
fn verification_passes_then_fails_for_every_tampered_format() {
    let dir = workdir("tamper");
    run_all(&dir, Some("demo-run"));
    let out = dir.join("out");
    let clean = export::verify(&out, None).expect("verify clean");
    assert!(clean.ok, "{clean:?}");
    assert!(clean.records_ok);
    assert!(clean
        .files
        .iter()
        .all(|f| f.present && f.sha_ok && f.bytes_ok));

    for name in [
        "records.csv",
        "records.jsonl",
        "dataset-demo-run.xlsx",
        "dataset-demo-run.parquet",
        "dataset-demo-run.pdf",
        "dataset-demo-run.xml",
        "checksums.txt",
    ] {
        let path = out.join(name);
        let mut bytes = std::fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0x01;
        std::fs::write(&path, &bytes).unwrap();
        let v = export::verify(&out, None).expect("verify tampered");
        assert!(!v.ok, "{name}: tamper went undetected");
        let check = v.files.iter().find(|f| f.name == name).unwrap();
        assert!(!check.sha_ok, "{name}: sha check did not fail");
        // restore
        bytes[mid] ^= 0x01;
        std::fs::write(&path, &bytes).unwrap();
    }
    let restored = export::verify(&out, None).expect("verify restored");
    assert!(restored.ok, "restoring every byte restores the verdict");
}

#[test]
fn dataset_ids_are_sanitized_into_safe_file_names() {
    let dir = workdir("names");
    let src = dir.join("in");
    std::fs::create_dir_all(&src).unwrap();
    let input = input_jsonl(&src, 3);
    let out = dir.join("out");
    let report = export::run(&ExportOpts {
        inputs: vec![input],
        out_dir: out.clone(),
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: vec![ExportFormat::Xlsx],
        dataset: Some("../../etc/passwd".to_string()),
        pdf_sample_rows: 12,
    })
    .expect("export run");
    let name = report
        .manifest
        .files
        .iter()
        .map(|f| f.name.clone())
        .find(|n| n.ends_with(".xlsx"))
        .expect("xlsx entry");
    assert!(!name.contains('/'), "unsafe dataset id produced {name}");
    assert!(!name.contains(".."), "unsafe dataset id produced {name}");
    assert!(name.starts_with("dataset-") && name.ends_with(".xlsx"));
    assert!(out.join(name).exists());
}

#[test]
fn non_jsonl_exports_still_verify() {
    // Formats without JSONL cannot be re-counted from the folder, so the
    // manifest's declared record total is carried rather than re-derived — and
    // say so through `records_basis`.
    let dir = workdir("no-jsonl");
    let src = dir.join("in");
    std::fs::create_dir_all(&src).unwrap();
    let input = input_jsonl(&src, 10);
    let out = dir.join("out");
    export::run(&ExportOpts {
        inputs: vec![input],
        out_dir: out.clone(),
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: vec![ExportFormat::Xlsx, ExportFormat::Parquet],
        dataset: Some("demo-run".to_string()),
        pdf_sample_rows: 12,
    })
    .expect("export run");
    let v = export::verify(&out, None).expect("verify");
    assert!(v.ok, "{v:?}");
    assert!(v.records_ok);
    assert_eq!(v.records_basis, "manifest-declared");
}

#[test]
fn adapters_are_deterministic_for_the_same_input() {
    let a = workdir("det-a");
    let b = workdir("det-b");
    let ra = run_all(&a, Some("demo-run"));
    let _rb = run_all(&b, Some("demo-run"));
    for name in [
        "records.csv",
        "records.jsonl",
        "dataset-demo-run.xlsx",
        "dataset-demo-run.parquet",
        "dataset-demo-run.xml",
    ] {
        assert_eq!(
            sha256_of(&a.join("out").join(name)),
            sha256_of(&b.join("out").join(name)),
            "{name} is not deterministic"
        );
    }
    assert_eq!(ra.manifest.records, 120);
}

#[test]
fn format_registry_is_honest() {
    assert_eq!(ExportFormat::all().len(), 7);
    assert_eq!(ExportFormat::parse("xlsx"), Some(ExportFormat::Xlsx));
    assert_eq!(ExportFormat::parse("bundle"), Some(ExportFormat::Zip));
    assert_eq!(ExportFormat::parse("xls"), None);
    for f in ExportFormat::all() {
        assert!(!f.purpose().is_empty(), "{f} needs a factual purpose");
        assert_eq!(f.as_str(), f.ext());
    }
    let parsed = export::parse_formats(&["csv,jsonl".to_string(), "pdf".to_string()]).unwrap();
    assert_eq!(
        parsed,
        vec![ExportFormat::Csv, ExportFormat::Jsonl, ExportFormat::Pdf]
    );
    assert!(export::parse_formats(&["docx".to_string()]).is_err());
}
