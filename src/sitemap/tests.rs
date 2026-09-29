use super::*;

#[test]
fn flat_urlset_with_entities() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
          <url><loc>https://x.example/a?u=1&amp;t=2</loc><lastmod>2026-01-01</lastmod></url>
          <url><loc>  https://x.example/b&#38;p=1  </loc></url>
          <url><loc>not a url</loc></url>
          <url><loc></loc></url>
        </urlset>"#;
    assert_eq!(
        parse(xml),
        Sitemap::Urls(vec![
            "https://x.example/a?u=1&t=2".into(),
            "https://x.example/b&p=1".into()
        ])
    );
}

#[test]
fn index_uppercase_and_cdata() {
    let xml = r#"<SitemapIndex><sitemap><loc><![CDATA[ https://x.example/sm1.xml ]]></loc></sitemap>
                 <sitemap><loc>https://x.example/sm2.xml</loc></sitemap></SitemapIndex>"#;
    assert_eq!(
        parse(xml),
        Sitemap::Index(vec![
            "https://x.example/sm1.xml".into(),
            "https://x.example/sm2.xml".into()
        ])
    );
}

#[test]
fn garbage_yields_empty_never_panics() {
    assert_eq!(parse("<urlset><loc>https:/x"), Sitemap::Urls(vec![]));
    assert_eq!(parse(""), Sitemap::Urls(vec![]));
    assert_eq!(parse("<<<>>>"), Sitemap::Urls(vec![]));
    assert_eq!(parse("<loc>&&amp&;<"), Sitemap::Urls(vec![])); // bad entity, no colon-scheme
}

#[test]
fn relative_locs_survive_for_the_caller_to_resolve() {
    let xml =
        "<urlset><url><loc>/a/b?x=1</loc></url><url><loc>https://q.example/c</loc></url></urlset>";
    assert_eq!(
        parse(xml),
        Sitemap::Urls(vec!["/a/b?x=1".into(), "https://q.example/c".into()])
    );
}

#[test]
fn unclosed_loc_is_dropped() {
    // The trailing <loc> with no </loc> must not swallow the document.
    let xml = "<urlset><url><loc>https://a.example/1</loc></url><url><loc>https://a.example/x</url></urlset>";
    assert_eq!(
        parse(xml),
        Sitemap::Urls(vec!["https://a.example/1".into()])
    );
}
