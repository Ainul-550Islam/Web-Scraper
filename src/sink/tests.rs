use super::*;

fn rec(k: &str, v: &str) -> Record {
    let mut f = Map::new();
    f.insert(k.into(), serde_json::Value::String(v.into()));
    Record {
        run: "r1".into(),
        target: "t".into(),
        url: "http://x/1".into(),
        depth: 0,
        ts: 1700000000,
        fingerprint: "abc".into(),
        egress: "direct".into(),
        fields: f,
    }
}

#[test]
fn jsonl_appends_and_is_parseable() {
    let dir = std::env::temp_dir().join(format!("harvest-test-{}", std::process::id()));
    let p = dir.join("out.jsonl");
    let _ = std::fs::remove_dir_all(&dir);
    let s = JsonlSink::open(&p).unwrap();
    s.write(&rec("title", "hello, \"world\"")).unwrap();
    s.write(&rec("title", "second")).unwrap();
    s.flush().unwrap();
    let txt = std::fs::read_to_string(&p).unwrap();
    let lines: Vec<&str> = txt.lines().collect();
    assert_eq!(lines.len(), 2);
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["fields"]["title"], "hello, \"world\"");
    let _ = std::fs::remove_dir_all(&dir);
}

fn stub_ingest() -> (
    String,
    std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) {
    use std::io::{BufRead, Read};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let got: std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>> = Default::default();
    let got2 = got.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming().take(10) {
            let Ok(mut s) = conn else { continue };
            let Ok(clone) = s.try_clone() else { continue };
            let mut reader = std::io::BufReader::new(clone);
            let mut req = String::new();
            let _ = reader.read_line(&mut req);
            let mut clen = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h == "\n" || h == "\r\n" {
                    break;
                }
                if h.to_lowercase().starts_with("content-length:") {
                    clen = h
                        .split_once(':')
                        .map(|(_, v)| v.trim().parse().unwrap_or(0))
                        .unwrap_or(0);
                }
            }
            let mut body = vec![0u8; clen];
            let _ = reader.read_exact(&mut body);
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                if let Some(arr) = v.as_array() {
                    got2.lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .extend(arr.iter().cloned());
                }
            }
            let ok = "{\"ok\":true}";
            let _ = s.write_all(
                format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{ok}", ok.len())
                    .as_bytes(),
            );
            let _ = s.flush();
        }
    });
    (format!("http://127.0.0.1:{port}/ingest"), got)
}

#[test]
fn http_sink_batches_and_flushes() {
    let (url, got) = stub_ingest();
    let sink = HttpSink::open(url, 5, Some("Bearer t".into()), 2).unwrap();
    sink.write(&rec("title", "1")).unwrap();
    assert_eq!(sink.buffered(), 1, "below batch size: stay buffered");
    assert!(
        got.lock().unwrap_or_else(|p| p.into_inner()).is_empty(),
        "no POST before batch fills"
    );
    sink.write(&rec("title", "2")).unwrap(); // batch of 2 fires
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(
        got.lock().unwrap_or_else(|p| p.into_inner()).len(),
        2,
        "batch POST should deliver both records"
    );
    sink.write(&rec("title", "3")).unwrap();
    sink.flush().unwrap(); // remainder forced out
    std::thread::sleep(std::time::Duration::from_millis(200));
    let g = got.lock().unwrap_or_else(|p| p.into_inner());
    assert_eq!(g.len(), 3, "flush must send the remainder");
    assert_eq!(g[2]["fields"]["title"], "3");
    assert_eq!(
        sink.batches_sent.load(std::sync::atomic::Ordering::Relaxed),
        2
    );
}

#[test]
fn dataset_partitions_by_utc_day() {
    // 1700000000 = 2023-11-14 22:13:20 UTC
    assert_eq!(civil_from_days(1700000000 / 86_400), (2023, 11, 14));
    let dir = std::env::temp_dir().join(format!("harvest-ds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s = DatasetSink::open(dir.clone(), "day").unwrap();
    let mut day1 = rec("title", "a");
    day1.ts = 1700000000;
    let mut day2 = rec("title", "b");
    day2.ts = 1700000000 + 86_400; // +1 day
    let mut day2b = rec("title", "c");
    day2b.ts = 1700000000 + 86_400 + 60;
    s.write(&day1).unwrap();
    s.write(&day2).unwrap();
    s.write(&day2b).unwrap();
    s.flush().unwrap();
    let mut parts = s.parts();
    parts.sort();
    assert_eq!(parts.len(), 2, "one file per dt partition");
    assert!(
        parts[0].starts_with(dir.join("dt=2023-11-14")),
        "{:?}",
        parts[0]
    );
    assert!(
        parts[1].starts_with(dir.join("dt=2023-11-15")),
        "{:?}",
        parts[1]
    );
    let txt2 = std::fs::read_to_string(&parts[1]).unwrap();
    assert_eq!(
        txt2.lines().count(),
        2,
        "same-day records share a part file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn csv_escapes_and_writes_header_once() {
    let dir = std::env::temp_dir().join(format!("harvest-csv-{}", std::process::id()));
    let p = dir.join("out.csv");
    let _ = std::fs::remove_dir_all(&dir);
    let s = CsvSink::open(&p).unwrap();
    s.write(&rec("title", "a,b")).unwrap();
    s.write(&rec("title", "c;d")).unwrap();
    s.write(&rec("title", "say \"hi\"")).unwrap(); // embedded quotes
    s.flush().unwrap();
    let txt = std::fs::read_to_string(&p).unwrap();
    let lines: Vec<&str> = txt.lines().collect();
    assert_eq!(lines.len(), 4, "header + 3 rows");
    assert!(lines[0].starts_with("run,target,url,depth,ts,fingerprint,egress,title"));
    assert!(txt.contains("\"a,b\""));
    assert!(
        txt.contains("\"say \"\"hi\"\"\""),
        "quotes must double per RFC 4180:\n{txt}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn civil_from_days_pins_calendar_corners() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(-1), (1969, 12, 31), "pre-epoch floor");
    assert_eq!(civil_from_days(59), (1970, 3, 1), "1970 is not a leap year");
    let leap = crate::transform::days_from_civil(2016, 2, 29);
    assert_eq!(civil_from_days(leap), (2016, 2, 29), "leap day exists");
    assert_eq!(civil_from_days(leap + 1), (2016, 3, 1));
    // Inverse property against the transform module's forward direction.
    for (y, m, d) in [
        (1900, 3, 1),
        (1999, 12, 31),
        (2000, 2, 29),
        (2100, 3, 1),
        (1970, 1, 1),
    ] {
        let days = crate::transform::days_from_civil(y, m, d);
        assert_eq!(civil_from_days(days), (y, m, d), "{y}-{m}-{d}");
    }
}

#[test]
fn csv_escape_and_json_flat_shapes() {
    assert_eq!(csv_escape("plain"), "plain");
    assert_eq!(csv_escape("a,b"), "\"a,b\"");
    assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    assert_eq!(csv_escape("line1\nline2"), "\"line1\nline2\"");
    assert_eq!(json_flat(&serde_json::json!("str")), "str");
    assert_eq!(json_flat(&serde_json::Value::Null), "");
    assert_eq!(json_flat(&serde_json::json!(42)), "42");
    // nested stays compact JSON, valid for re-parsing
    let nested = json_flat(&serde_json::json!({"a": [1, 2]}));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&nested).unwrap()["a"][1],
        2
    );
}

#[test]
fn null_sink_is_total_and_reports_placeholder_path() {
    let (sink, path) = build(&crate::config::SinkCfg::Null {}).unwrap();
    assert_eq!(path.to_str().unwrap(), "(null)");
    sink.write(&rec("k", "v")).unwrap();
    sink.flush().unwrap();
}

#[test]
fn jsonl_sink_creates_parent_dirs() {
    let dir = std::env::temp_dir().join(format!("harvest-sink-mk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("deep/deeper/out.jsonl");
    let (sink, _) = build(&crate::config::SinkCfg::Jsonl { path: path.clone() })
        .expect("jsonl sink must mkdir -p its own output");
    sink.write(&rec("t", "1")).unwrap();
    sink.flush().unwrap();
    let txt = std::fs::read_to_string(&path).unwrap();
    assert_eq!(txt.lines().count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_csv_header_stays_aligned_for_different_targets_and_reopen() {
    let dir = std::env::temp_dir().join(format!("harvest-audit-csv-{}", std::process::id()));
    let p = dir.join("out.csv");
    let _ = std::fs::remove_dir_all(&dir);
    let s = CsvSink::open(&p).unwrap();
    let mut first = rec("a", "A");
    first.fields.insert("b".into(), serde_json::json!("B"));
    s.write(&first).unwrap();
    s.write(&rec("b", "second B")).unwrap();
    s.flush().unwrap();
    drop(s);
    let s = CsvSink::open(&p).unwrap();
    s.write(&rec("b", "third B")).unwrap();
    s.flush().unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    let mut rows = csv::Reader::from_reader(text.as_bytes());
    let records: Vec<_> = rows.records().map(Result::unwrap).collect();
    assert_eq!(&records[1][7], "");
    assert_eq!(&records[1][8], "second B");
    assert_eq!(&records[2][8], "third B");
    assert!(s
        .write(&rec("new-column", "refuse, never corrupt"))
        .is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn audit_csv_quotes_carriage_returns_and_neutralizes_formulas() {
    assert_eq!(csv_escape("a\rb"), "\"a\rb\"");
    assert_eq!(
        crate::export::spreadsheet_text("=HYPERLINK(\"https://evil.test\")"),
        "'=HYPERLINK(\"https://evil.test\")"
    );
}

#[cfg(unix)]
#[test]
fn phase2_jsonl_output_cannot_append_through_a_symlink() {
    let t = std::env::temp_dir().join(format!("harvest-phase2-sink-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(&t).unwrap();
    let original = t.join("private");
    std::fs::write(&original, b"keep").unwrap();
    let link = t.join("output.jsonl");
    std::os::unix::fs::symlink(&original, &link).unwrap();
    assert!(
        JsonlSink::open(&link).is_err(),
        "output must not adopt a symlinked file"
    );
    assert_eq!(std::fs::read(original).unwrap(), b"keep");
    std::fs::remove_dir_all(t).unwrap();
}

#[cfg(unix)]
#[test]
fn phase2_sink_creation_rejects_a_symlinked_parent_before_making_directories() {
    let t = std::env::temp_dir().join(format!("harvest-phase2-parent-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    std::fs::create_dir_all(t.join("real")).unwrap();
    let alias = t.join("alias");
    std::os::unix::fs::symlink(t.join("real"), &alias).unwrap();
    let output = alias.join("nested/output.jsonl");
    assert!(JsonlSink::open(&output).is_err());
    assert!(!t.join("real/nested").exists());
    std::fs::remove_dir_all(t).unwrap();
}

#[test]
fn phase2_csv_schema_refuses_formula_protected_header_collisions() {
    let t = std::env::temp_dir().join(format!(
        "harvest-phase2-csv-alias-{}.csv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&t);
    let result = CsvSink::open_with_fields(&t, Some(vec!["=price".into(), "'=price".into()]));
    assert!(
        result.is_err(),
        "two logical keys must not silently share one protected CSV heading"
    );
    let _ = std::fs::remove_file(t);
}
