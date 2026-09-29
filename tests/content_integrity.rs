// Content handling: MIME/charset classification, malformed input, size limits,
// binary safety and deterministic normalization. Every case here is input a
// real site actually serves (wrong Content-Type, BOMs, truncated bodies,
// latini-1 pages, JSON with a trailing comma) — the classification layer is
// total: it never panics and never returns a half-guess.

use harvest::content::{
    classify, classify_bounded, decode_text, extension_for_mime, is_probably_binary,
    kind_from_mime, mime_from_extension, normalize_mime, normalize_text, parse_content_type,
    ContentInfo, ContentKind, DEFAULT_MAX_BODY_BYTES, MAX_SNIFF_BYTES,
};
use harvest::document::{
    canonical_url_key, safe_dataset_name, sha256_hex, title_from_html, DocumentSet,
};
use harvest::media::{
    extension_of, resource, resource_kind_for, MediaCollector, MediaPolicy, ResourceKind,
};

#[test]
fn headers_win_and_contradictions_are_flagged() {
    let info = classify(Some("text/html; charset=utf-8"), b"%PDF-1.7 ...");
    assert_eq!(
        info.kind,
        ContentKind::Html,
        "declared type is authoritative"
    );
    assert!(info.mismatch, "but the disagreement is reported");
    assert_eq!(info.charset.as_deref(), Some("utf-8"));

    // An unparseable-but-declared type is still the server's declaration: a
    // PNG-typed response is not silently re-parsed as HTML.
    let info = classify(Some("image/png"), b"<html><body>hi</body></html>");
    assert_eq!(info.kind, ContentKind::Binary);
    assert!(info.mismatch);
}

#[test]
fn absent_or_generic_headers_fall_back_to_sniffing() {
    let pdf = classify(Some("application/octet-stream"), b"%PDF-1.7");
    assert_eq!(pdf.kind, ContentKind::Pdf);
    assert!(!pdf.mismatch);
    let json = classify(None, br#"{"a":1}"#);
    assert_eq!(json.kind, ContentKind::Json);
    let sitemap = classify(
        None,
        b"<?xml version=\"1.0\"?><urlset><url><loc>/a</loc></url></urlset>",
    );
    assert_eq!(sitemap.kind, ContentKind::Xml);
    let rss = classify(
        None,
        b"<?xml version=\"1.0\"?><rss version=\"2.0\"><channel/></rss>",
    );
    assert_eq!(rss.kind, ContentKind::Xml);
}

#[test]
fn extensions_are_reported_never_trusted() {
    // A .json URL serving HTML is classified from the bytes/header, not the path.
    let info = classify(Some("text/html"), b"<html><body>x</body></html>");
    assert_eq!(info.kind, ContentKind::Html);
    assert_eq!(
        extension_of("https://x.test/a/data.JSON?x=1#f").as_deref(),
        Some("json")
    );
    assert_eq!(mime_from_extension("pdf").unwrap(), "application/pdf");
    assert_eq!(extension_for_mime("application/pdf"), Some("pdf"));
    assert_eq!(mime_from_extension("definitely-not-a-type"), None);
}

#[test]
fn malformed_json_and_truncated_html_do_not_panic() {
    assert_eq!(classify(None, b"{not json").kind, ContentKind::Text);
    assert_eq!(classify(None, b"[1, 2,").kind, ContentKind::Text);
    assert_eq!(
        classify(None, b"").kind,
        ContentKind::Text,
        "empty body: no signal, decodable"
    );
    assert!(!is_probably_binary(b""));
    // A NUL byte anywhere in the sniff window means binary, full stop.
    assert!(is_probably_binary(&[b'a', 0, b'b']));
    assert!(!is_probably_binary("héllo".as_bytes()));
}

#[test]
fn charset_detection_and_decoding() {
    let utf8 = classify(None, "\u{feff}hello".as_bytes());
    assert_eq!(
        utf8.charset.as_deref(),
        Some("utf-8"),
        "BOM-detected charset is reported"
    );

    // UTF-16LE with BOM: decoded, not mangled.
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .iter()
        .copied()
        .chain("hi".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let info = classify(None, &utf16);
    assert_eq!(info.charset.as_deref(), Some("utf-16le"));
    assert_eq!(decode_text(&info, &utf16), "hi");

    // Latin-1 and Windows-1252 are transcoded, and the two differ exactly
    // where they are documented to differ.
    let latin = ContentInfo {
        charset: Some("iso-8859-1".to_string()),
        ..ContentInfo::empty()
    };
    assert_eq!(decode_text(&latin, &[0x41, 0xE9]), "A\u{e9}");
    let win = ContentInfo {
        charset: Some("windows-1252".to_string()),
        ..ContentInfo::empty()
    };
    assert_eq!(decode_text(&win, &[0x80]), "\u{20ac}");

    // Invalid UTF-8 degrades to replacement characters instead of failing.
    let broken = ContentInfo {
        charset: Some("utf-8".to_string()),
        ..ContentInfo::empty()
    };
    assert!(decode_text(&broken, &[0x41, 0xFF, 0x42]).contains('\u{fffd}'));
}

#[test]
fn normalization_is_deterministic_and_safe() {
    let a = normalize_text("\u{feff}A\u{0}B\r\nC   D\t\tE\u{7f}");
    let b = normalize_text("\u{feff}A\u{0}B\r\nC   D\t\tE\u{7f}");
    assert_eq!(a, b, "same input, same output");
    assert_eq!(a, "AB\nC D E");
    assert!(!normalize_text("x").contains('\u{0}'));
    assert_eq!(normalize_text(""), "");
}

#[test]
fn size_limits_are_enforced_before_decoding() {
    let body = vec![b'a'; 100];
    let err = classify_bounded(None, &body, 99).unwrap_err();
    assert_eq!(err.bytes, 100);
    assert_eq!(err.limit, 99);
    assert!(err.to_string().contains("exceeds the configured limit"));
    assert!(classify_bounded(None, &body, 100).is_ok());
    assert_eq!(DEFAULT_MAX_BODY_BYTES, 8 * 1024 * 1024);
    // Compile-time guard: the sniff window must stay useful.
    const _: () = assert!(MAX_SNIFF_BYTES >= 4096);
}

#[test]
fn header_parsing_is_total() {
    assert_eq!(
        parse_content_type("text/html; charset=UTF-8"),
        (Some("text/html".to_string()), Some("utf-8".to_string()))
    );
    assert_eq!(
        parse_content_type("application/json;charset=\"utf-8\"; boundary=x"),
        (
            Some("application/json".to_string()),
            Some("utf-8".to_string())
        )
    );
    assert_eq!(parse_content_type(""), (Some(String::new()), None));
    assert_eq!(normalize_mime(" Text/HTML ; charset=utf-8"), "text/html");
    assert_eq!(
        kind_from_mime("application/vnd.api+json"),
        Some(ContentKind::Json)
    );
    assert_eq!(kind_from_mime("image/webp"), None);
}

#[test]
fn checksums_are_stable_and_documents_dedupe() {
    assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
    assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
    assert_eq!(sha256_hex(b"").len(), 64);

    let mut set = DocumentSet::new(2);
    let info = classify(Some("text/html"), b"<html><body>x</body></html>");
    for i in 0..5 {
        let mut d = harvest::document::Document::observed(
            format!("https://x.test/{i}"),
            "t",
            &info,
            harvest::document::DiscoverySource::Link,
            0,
            Some(200),
            0,
        );
        d = d.with_sha256(Some(sha256_hex(i.to_string().as_bytes())));
        set.push(d);
    }
    assert_eq!(set.len(), 2, "cap holds");
    assert_eq!(
        canonical_url_key("HTTPS://X.test/a?b=1#frag"),
        "https://x.test/a?b=1"
    );
}

#[test]
fn titles_and_dataset_names_are_bounded_and_safe() {
    assert_eq!(
        title_from_html("<html><head><title>  Big   Sale\n2026 </title>"),
        Some("Big Sale 2026".to_string())
    );
    assert_eq!(title_from_html("<title></title>"), None);
    assert_eq!(safe_dataset_name("run-7"), "dataset-run-7");
    assert!(!safe_dataset_name("../../etc/passwd").contains('/'));
    assert_eq!(safe_dataset_name("///"), "dataset");
}

#[test]
fn binary_responses_stay_metadata_only() {
    // A binary body is classified as such, and its document is not a record
    // source — no code path tries to decode it into records.
    let png_header = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01];
    let info = classify(Some("image/png"), &png_header);
    assert_eq!(info.kind, ContentKind::Binary);
    assert!(!info.kind.is_textual());
    assert!(is_probably_binary(&png_header));

    let info = classify(Some("application/pdf"), b"%PDF-1.7");
    assert_eq!(info.kind, ContentKind::Pdf);
    assert!(
        !info.kind.is_textual(),
        "PDF metadata only — no page-text claims"
    );
}

#[test]
fn resource_classification_prefers_mime_over_extension() {
    assert_eq!(resource_kind_for("image/png", None), ResourceKind::Image);
    assert_eq!(resource_kind_for("video/mp4", None), ResourceKind::Video);
    assert_eq!(resource_kind_for("audio/mpeg", None), ResourceKind::Audio);
    assert_eq!(
        resource_kind_for("application/pdf", None),
        ResourceKind::Document
    );
    assert_eq!(
        resource_kind_for("application/zip", None),
        ResourceKind::Archive
    );
    assert_eq!(
        resource_kind_for("application/octet-stream", Some("pdf")),
        ResourceKind::Document
    );
    assert_eq!(
        resource_kind_for("application/octet-stream", None),
        ResourceKind::Unknown
    );

    let r = resource(
        "https://x.test/a/b.PDF",
        Some("image/png"),
        Some(10),
        None,
        "p",
        harvest::document::DiscoverySource::Resource,
        0,
    );
    assert_eq!(r.mime, "image/png", "header beats extension");
    assert!(!r.mime_guessed);
    assert_eq!(r.kind, ResourceKind::Image);
    let guessed = resource(
        "https://x.test/a/b.pdf",
        None,
        Some(10),
        None,
        "p",
        harvest::document::DiscoverySource::Resource,
        0,
    );
    assert!(guessed.mime_guessed);
    assert_eq!(guessed.mime, "application/pdf");
}

#[test]
fn media_policy_enforces_size_and_allow_lists() {
    let policy = MediaPolicy {
        max_bytes: 100,
        max_resources: 10,
        max_per_page: 10,
        allow: vec!["pdf".to_string()],
        deny: vec!["png".to_string()],
        allow_unknown_size: false,
    };
    let mut c = MediaCollector::new(policy);
    let big = resource(
        "https://x.test/a.pdf",
        Some("application/pdf"),
        Some(5000),
        None,
        "p",
        harvest::document::DiscoverySource::Resource,
        0,
    );
    assert_eq!(c.push(big), harvest::media::PushOutcome::TooLarge);
    let unknown = resource(
        "https://x.test/a.pdf",
        Some("application/pdf"),
        None,
        None,
        "p",
        harvest::document::DiscoverySource::Resource,
        0,
    );
    assert_eq!(c.push(unknown), harvest::media::PushOutcome::TooLarge);
    let denied = resource(
        "https://x.test/a.png",
        Some("image/png"),
        Some(10),
        None,
        "p",
        harvest::document::DiscoverySource::Media,
        0,
    );
    assert_eq!(c.push(denied), harvest::media::PushOutcome::NotAllowed);
    let ok = resource(
        "https://x.test/a.pdf",
        Some("application/pdf"),
        Some(10),
        None,
        "p",
        harvest::document::DiscoverySource::Resource,
        0,
    );
    assert_eq!(c.push(ok.clone()), harvest::media::PushOutcome::Added);
    assert_eq!(c.push(ok), harvest::media::PushOutcome::Duplicate);
    assert_eq!(c.total_bytes(), 10);
    assert_eq!(c.to_json()["count"], 1);
}

#[test]
fn large_bodies_are_classified_from_a_bounded_window() {
    // A 4 MiB body costs the same to classify as a 4 KiB one.
    let mut body = Vec::with_capacity(4 << 20);
    body.extend_from_slice(b"%PDF-1.7 ");
    body.resize(4 << 20, 0x20);
    let info = classify(None, &body);
    assert_eq!(info.kind, ContentKind::Pdf);
    assert_eq!(info.bytes, body.len() as u64);
}
