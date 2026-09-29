//! Zero-dependency throughput measurements (no criterion: the point is
//! *measured*, not tuned — these numbers exist so regressions are visible).
//! `cargo bench` runs them; they print ns/op tables. Nothing here asserts a
//! threshold: numbers depend on the machine, so they are for comparison on
//! one box only.

use std::hint::black_box;
use std::time::Instant;

fn bench(name: &str, iters: u64, mut f: impl FnMut(u64)) {
    // warm-up page (allocator + caches) excluded from timing
    for i in 0..(iters / 20).max(1) {
        f(i);
    }
    let t = Instant::now();
    for i in 0..iters {
        f(i);
    }
    let ns = t.elapsed().as_nanos() as f64 / iters as f64;
    println!("{name:<34} {ns:>10.1} ns/op   ({iters} iters)");
}

fn main() {
    // `cargo bench` already builds with optimizations; debug runs are ~100x
    // slower, so numbers recorded from `cargo bench` are the reference.
    println!(
        "harvest throughput bench v{} — machine-relative; do not compare boxes",
        env!("CARGO_PKG_VERSION")
    );

    // ---- id validation (every queue/API boundary parses these) ----
    let good_id = format!("{:064x}", 0x1234u64);
    bench("ids::RecordId::parse (valid)", 2_000_000, |_| {
        black_box(harvest::ids::RecordId::parse(black_box(&good_id)).unwrap());
    });
    let bad_id = "nope".repeat(8);
    bench("ids::RecordId::parse (reject)", 2_000_000, |_| {
        black_box(harvest::ids::RecordId::parse(black_box(&bad_id)).is_err());
    });
    let slug = "shop-catalog.a_1";
    bench("ids::TargetId::parse", 2_000_000, |_| {
        black_box(harvest::ids::TargetId::parse(black_box(slug)).unwrap());
    });

    // ---- budget hot path (checked per record write) ----
    let b = std::sync::Arc::new(harvest::budget::Budget::new("bench", u64::MAX));
    bench("budget::try_take (unlimited)", 20_000_000, |_| {
        let n = b.try_take(1);
        b.release(1);
        black_box(n);
    });

    // ---- metrics counters (every request touches these) ----
    let m = harvest::metrics::Metrics::default();
    bench("metrics::request_ok", 20_000_000, |_| {
        m.request_ok(); // atomics: side-effecting, not elidable
    });
    m.note_target_record("shop");
    bench("metrics::render_prometheus", 20_000, |_| {
        black_box(m.render_prometheus().len());
    });

    // ---- extract + transform pipeline (per page) ----
    let rules: std::collections::BTreeMap<String, harvest::config::FieldCfg> =
        serde_json::from_value(serde_json::json!({
            "title": { "css": "h1" },
            "price": { "css": "p.price" },
        }))
        .unwrap();
    let ex = harvest::extract::Extractor::compile(&rules).unwrap();
    let target = harvest::config::TargetCfg {
        extract: rules,
        ..Default::default()
    };
    let pipe = harvest::transform::TargetPipeline::compile(&target).unwrap();
    let html = format!(
        "<html><body><h1>Widget</h1><p class=price>$9.99</p>{}</body></html>",
        "x".repeat(2048)
    );
    let doc = scraper::Html::parse_document(&html);
    bench("extract::Extractor::run", 200_000, |_| {
        black_box(ex.extract(black_box(&doc)));
    });
    let base_fields = ex.extract(&doc);
    bench("transform::pipeline::apply", 100_000, |_| {
        let mut f = base_fields.clone(); // extraction excluded: this measures apply
        black_box(
            pipe.apply(black_box(&mut f), "https://x.test/p", 0)
                .unwrap(),
        );
    });

    // ---- export part chunking (delivery path) ----
    let dir = std::env::temp_dir().join(format!("harvest-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("records.jsonl");
    let mut big = String::new();
    for i in 0..5_000u64 {
        big.push_str(
            &serde_json::json!({
                "run": "bench", "target": "shop",
                "url": format!("https://x.test/{i}"), "depth": 0, "ts": i,
                "fingerprint": format!("{:064x}", i), "egress": "direct",
                "fields": { "title": format!("page {i}"), "price": i },
            })
            .to_string(),
        );
        big.push('\n');
    }
    std::fs::write(&input, &big).unwrap();
    let out = dir.join("exp");
    bench("export::run (5k records, chunked)", 20, |_| {
        let o = harvest::export::ExportOpts {
            inputs: vec![input.clone()],
            out_dir: out.clone(),
            split_target: false,
            history: None,
            key: None,
            max_part_bytes: 262_144,
            resume: false,
            formats: Vec::new(),
            dataset: None,
            pdf_sample_rows: 12,
        };
        black_box(harvest::export::run(&o).unwrap().manifest.records);
    });
    bench("export::verify", 50, |_| {
        black_box(harvest::export::verify(&out, None).unwrap().ok);
    });
    let _ = std::fs::remove_dir_all(&dir);

    println!("done — record these numbers per machine; do not compare boxes");
}
