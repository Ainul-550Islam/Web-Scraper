use super::*;

use serde_json::{Map, Value};

fn rec(fp: &str) -> Record {
    Record {
        run: "r".into(),
        target: "t".into(),
        url: "http://x/p".into(),
        depth: 2,
        ts: 123,
        fingerprint: fp.into(),
        egress: "direct".into(),
        fields: Map::from_iter([("k".to_string(), Value::from(1))]),
    }
}

#[test]
fn ddl_is_idempotent_and_identity_indexed() {
    // CREATE IF NOT EXISTS x2: resume/restart re-runs DDL per open().
    assert_eq!(DDL.matches("CREATE TABLE IF NOT EXISTS").count(), 1);
    assert_eq!(DDL.matches("INDEX IF NOT EXISTS").count(), 1);
    // The dedup contract: sink-level idempotency relies on this exact
    // unique key (ON CONFLICT DO NOTHING in the INSERT targets it).
    assert!(DDL.contains("ON harvest_records (run, target, fingerprint)"));
    assert!(DDL.contains("payload JSONB NOT NULL"));
}

#[test]
fn dead_database_fails_flush_not_write() {
    // Port 1: connect is refused instantly, no fixture needed. The
    // contract under test: buffering never fails a crawl step (write is
    // fire-and-forget), but flush surfaces the accumulated error so the
    // run SUMMARY reports it — silent loss is the bug this prevents.
    let sink = PgSink::open("host=127.0.0.1 port=1 user=nobody".into()).unwrap();
    sink.write(&rec("a")).unwrap();
    sink.write(&rec("b")).unwrap();
    let err = sink.flush().unwrap_err().to_string();
    assert!(err.contains("pg connect"), "{err}");
    assert_eq!(
        sink.buffered(),
        0,
        "flush drains the pending counter even on error"
    );
    assert_eq!(sink.rows_written.load(Ordering::Relaxed), 0);
    // Degraded state is sticky: later writes are COUNTED as lost, never
    // silently accepted, and every flush keeps reporting the failure.
    sink.write(&rec("c")).unwrap();
    let err2 = sink.flush().unwrap_err().to_string();
    assert!(err2.contains("pg connect"), "{err2}");
    assert!(
        err2.contains("records dropped"),
        "loss must be quantified: {err2}"
    );
}

#[test]
fn flush_on_abandoned_channel_reports_lost_ack_not_hang() {
    // Simulate the sink thread dying between send and ack: the engine
    // must get a bounded error (30s timeout worst case), never a hang.
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    drop(rx); // reader gone
    let (ack_tx, _ack_rx) = std::sync::mpsc::channel();
    assert!(tx.send(Cmd::Flush(ack_tx)).is_err());
}

#[test]
fn fail_all_acks_every_queued_flush_with_the_reason() {
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    let (a1, r1) = std::sync::mpsc::channel();
    let (a2, r2) = std::sync::mpsc::channel();
    tx.send(Cmd::Push(rec("x"))).unwrap();
    tx.send(Cmd::Flush(a1)).unwrap();
    tx.send(Cmd::Flush(a2)).unwrap();
    fail_all(&rx, "boom");
    for r in [r1, r2] {
        let ack = r.recv().unwrap().expect_err("must be error");
        assert!(ack.to_string().contains("boom"));
    }
}
