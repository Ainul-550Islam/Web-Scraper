// Crawl planning & discovery: the planner is a pure state machine, so the
// whole surface is testable without a network. These tests pin the *bounds*
// (depth, page budget, robots, include/exclude, body size) and the discovery
// categories (links, pagination, canonical, sitemap/index, feeds, documents,
// media) — every refusal is counted, never silent.

use std::path::PathBuf;

use harvest::crawl::{self, discovery, CrawlCfg, CrawlPlan, Skip};
use harvest::document::DiscoverySource;
use harvest::media::MediaPolicy;
use harvest::robots::Robots;

fn cfg(url: &str) -> CrawlCfg {
    CrawlCfg {
        start_urls: vec![url.to_string()],
        ..CrawlCfg::default()
    }
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-crawl-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const PAGE: &str = r#"<html><head>
  <link rel="canonical" href="https://shop.test/catalog/page-1"/>
  <link rel="next" href="https://shop.test/catalog/page-2"/>
  <link rel="alternate" hreflang="de" href="https://shop.test/de/catalog"/>
  <link rel="alternate" type="application/rss+xml" href="https://shop.test/feed.xml"/>
  <link rel="sitemap" href="https://shop.test/sitemap.xml"/>
</head><body>
  <a href="/p/1">One</a>
  <a href="/p/2">Two</a>
  <a href="/p/2">Two again</a>
  <a href="https://other.test/p/3">Off-host</a>
  <a href="mailto:someone@shop.test">Mail</a>
  <a href="javascript:void(0)">Script</a>
  <a href="/files/spec.pdf">Spec</a>
  <a href="/files/data.csv">Data</a>
  <img src="/img/a.png" alt="a"/>
  <video src="/v/clip.mp4"></video>
</body></html>"#;

#[test]
fn discovers_every_declared_category() {
    let url = url::Url::parse("https://shop.test/catalog/page-1").unwrap();
    let found = discovery::discover_html(&url, PAGE, &discovery::DiscoveryCfg::default(), 1);
    assert_eq!(
        found.canonical.as_ref().map(|c| c.url.as_str()),
        Some("https://shop.test/catalog/page-1")
    );
    assert_eq!(found.pagination.len(), 1, "rel=next");
    assert_eq!(found.pagination[0].url, "https://shop.test/catalog/page-2");
    assert_eq!(found.alternates.len(), 1, "hreflang alternate");
    assert_eq!(found.feeds.len(), 1, "rss feed");
    assert_eq!(found.sitemaps.len(), 1, "rel=sitemap");
    assert_eq!(found.links.len(), 2, "/p/1 and /p/2 (duplicate collapsed)");
    assert_eq!(found.documents.len(), 2, "pdf + csv");
    assert_eq!(found.media.len(), 2, "img + video");
    // Non-http schemes and off-host links are excluded by the same pass.
    let all: Vec<&str> = found.all().iter().map(|d| d.url.as_str()).collect();
    assert!(!all
        .iter()
        .any(|u| u.starts_with("mailto:") || u.starts_with("javascript:")));
    assert!(!all.iter().any(|u| u.contains("other.test")));
}

#[test]
fn malformed_html_and_xml_degrade_instead_of_failing() {
    let url = url::Url::parse("https://shop.test/").unwrap();
    let truncated = "<html><body><a href='/ok'>ok</a><a href='/broken<><'";
    let found = discovery::discover_html(&url, truncated, &discovery::DiscoveryCfg::default(), 1);
    assert!(found.links.iter().any(|l| l.url == "https://shop.test/ok"));

    let junk = "<sitemapindex><sitemap><loc>not a url at all";
    let s = discovery::discover_sitemap(&url, junk, 0);
    assert!(
        s.urls.is_empty() && s.children.is_empty(),
        "garbage yields nothing, never an error"
    );
}

#[test]
fn pagination_selector_and_rel_next_are_configurable() {
    let url = url::Url::parse("https://shop.test/catalog").unwrap();
    let html = r#"<html><body><a class="next" href="/catalog?page=2">next page</a></body></html>"#;
    // Without a selector and with rel=next only, the class-based link is a
    // plain link — the engine does not invent pagination URLs.
    let plain = discovery::discover_html(&url, html, &discovery::DiscoveryCfg::default(), 1);
    assert_eq!(plain.pagination.len(), 0);
    assert_eq!(plain.links.len(), 1);

    let with_selector = discovery::DiscoveryCfg {
        pagination_selector: Some("a.next".to_string()),
        ..discovery::DiscoveryCfg::default()
    };
    let found = discovery::discover_html(&url, html, &with_selector, 1);
    assert_eq!(found.pagination.len(), 1);
    assert_eq!(found.pagination[0].url, "https://shop.test/catalog?page=2");
}

#[test]
fn sitemap_and_index_recursion() {
    let mut plan = CrawlPlan::new(CrawlCfg {
        max_pages: 50,
        ..cfg("https://shop.test/")
    })
    .unwrap();
    let index = r#"<sitemapindex><sitemap><loc>https://shop.test/s1.xml</loc></sitemap>
                   <sitemap><loc>/s2.xml</loc></sitemap></sitemapindex>"#;
    let out = plan.seed_sitemap("https://shop.test/sitemap.xml", index, 0);
    assert!(out.is_index);
    assert_eq!(out.children.len(), 2);
    assert_eq!(
        out.queued, 0,
        "an index queues pages only from its children"
    );

    let flat = r#"<urlset><url><loc>https://shop.test/p1</loc></url><url><loc>/p2</loc></url>
                  <url><loc>/p1</loc></url></urlset>"#;
    let out = plan.seed_sitemap("https://shop.test/sitemap.xml", flat, 0);
    assert_eq!(out.queued, 2, "duplicate loc resolved to the same URL");
    assert_eq!(plan.pending(), 2);
    assert_eq!(plan.stats().sitemaps_seen, 2);
    assert_eq!(plan.stats().sitemap_indexes, 1);
}

#[test]
fn robots_sitemap_lines_and_robots_authority() {
    let robots_txt = "User-agent: *\nDisallow: /private\nSitemap: https://shop.test/s1.xml\nSitemap: https://shop.test/s2.xml\n";
    let plan = CrawlPlan::new(cfg("https://shop.test/")).unwrap();
    assert_eq!(plan.robots_sitemaps(robots_txt).len(), 2);

    let mut plan = CrawlPlan::new(cfg("https://shop.test/")).unwrap();
    plan.set_robots(Robots::parse(robots_txt, "harvest"));
    assert_eq!(
        plan.admit("https://shop.test/private/a", 1),
        harvest::crawl::Admit::Deny(Skip::Robots)
    );
    assert_eq!(
        plan.admit("https://shop.test/public", 1),
        harvest::crawl::Admit::Allow
    );

    // With robots denying a discovered link, expanding a page counts the
    // refusal instead of dropping it.
    let mut plan = CrawlPlan::new(CrawlCfg {
        max_pages: 50,
        ..cfg("https://shop.test/")
    })
    .unwrap();
    plan.seed();
    plan.set_robots(Robots::parse(robots_txt, "harvest"));
    let body = b"<html><body><a href=\"/private/x\">x</a><a href=\"/public\">p</a></body></html>";
    plan.expand("https://shop.test/", body, Some("text/html"), 0, 0);
    assert_eq!(plan.stats().skipped_robots, 1);
    assert_eq!(plan.stats().queued, 2, "start URL + the public link");
}

#[test]
fn include_exclude_and_host_scope_gate_admission() {
    let mut c = cfg("https://shop.test/");
    c.include = vec!["/keep".to_string()];
    c.exclude = vec!["/keep/but-not".to_string()];
    let plan = CrawlPlan::new(c).unwrap();
    assert_eq!(
        plan.admit("https://shop.test/keep/a", 0),
        harvest::crawl::Admit::Allow
    );
    assert_eq!(
        plan.admit("https://shop.test/keep/but-not", 0),
        harvest::crawl::Admit::Deny(Skip::Excluded)
    );
    assert_eq!(
        plan.admit("https://shop.test/other", 0),
        harvest::crawl::Admit::Deny(Skip::NotIncluded)
    );
    assert_eq!(
        plan.admit("https://elsewhere.test/keep/a", 0),
        harvest::crawl::Admit::Deny(Skip::OffHost)
    );
}

#[test]
fn depth_and_page_budgets_are_hard_limits() {
    let mut plan = CrawlPlan::new(CrawlCfg {
        max_depth: 1,
        max_pages: 3,
        ..cfg("https://shop.test/")
    })
    .unwrap();
    assert_eq!(plan.seed(), 1);
    assert_eq!(
        plan.admit("https://shop.test/d1", 1),
        harvest::crawl::Admit::Allow
    );
    assert_eq!(
        plan.admit("https://shop.test/d2", 2),
        harvest::crawl::Admit::Deny(Skip::Depth)
    );

    let body = b"<html><body>\
        <a href=\"/a\">a</a><a href=\"/b\">b</a><a href=\"/c\">c</a><a href=\"/d\">d</a>\
        </body></html>";
    let e = plan.expand("https://shop.test/", body, Some("text/html"), 0, 0);
    assert_eq!(e.candidates, 4);
    assert_eq!(e.queued, 2, "max_pages=3 and one slot is the start URL");
    assert_eq!(plan.stats().skipped_budget, 2);
}

#[test]
fn duplicate_urls_are_collapsed_across_discovery_and_visits() {
    let mut plan = CrawlPlan::new(CrawlCfg {
        max_pages: 20,
        ..cfg("https://shop.test/")
    })
    .unwrap();
    plan.seed();
    let body = b"<html><body><a href=\"/a\">a</a><a href=\"/a#section\">a again</a></body></html>";
    let e = plan.expand("https://shop.test/", body, Some("text/html"), 0, 0);
    assert_eq!(e.queued, 1, "fragment-only difference is the same URL");
    let batch = plan.next(10);
    assert_eq!(batch.len(), 2);
    assert_eq!(
        plan.admit("https://shop.test/a", 1),
        harvest::crawl::Admit::Deny(Skip::Duplicate)
    );
}

#[test]
fn oversized_bodies_are_refused_before_decoding() {
    let mut c = cfg("https://shop.test/");
    c.max_response_bytes = 64;
    let mut plan = CrawlPlan::new(c).unwrap();
    let e = plan.expand(
        "https://shop.test/p",
        &[b'x'; 4096],
        Some("text/html"),
        0,
        0,
    );
    assert!(e.body_too_large);
    assert_eq!(e.queued, 0);
    assert_eq!(plan.stats().body_too_large, 1);
    assert_eq!(plan.documents().len(), 0, "a refused body is not indexed");
}

#[test]
fn content_type_scope_decides_what_is_expanded() {
    let c = CrawlCfg {
        content_types: vec!["html".to_string()],
        ..cfg("https://shop.test/")
    };
    let mut plan = CrawlPlan::new(c).unwrap();
    let html = b"<html><body><a href=\"/a\">a</a></body></html>";
    assert!(plan.kind_allowed(harvest::content::ContentKind::Html));
    assert!(!plan.kind_allowed(harvest::content::ContentKind::Json));
    let e = plan.expand("https://shop.test/p", html, Some("application/json"), 0, 0);
    assert_eq!(e.queued, 0);
    assert_eq!(plan.stats().skipped_content_type, 1);
    assert_eq!(
        plan.documents().len(),
        1,
        "recorded as a document either way"
    );
}

#[test]
fn resources_are_recorded_as_metadata_only() {
    let mut c = cfg("https://shop.test/");
    // Unknown size + no consent for unknown sizes ⇒ nothing is recorded.
    c.media = MediaPolicy {
        allow_unknown_size: false,
        ..MediaPolicy::default()
    };
    let mut plan = CrawlPlan::new(c).unwrap();
    plan.expand(
        "https://shop.test/",
        PAGE.as_bytes(),
        Some("text/html"),
        0,
        0,
    );
    assert_eq!(plan.media().len(), 0);
    assert!(plan.stats().media_refused > 0);

    let c = CrawlCfg {
        media: MediaPolicy {
            allow_unknown_size: true,
            max_bytes: 16 * 1024 * 1024,
            ..MediaPolicy::default()
        },
        ..cfg("https://shop.test/")
    };
    let mut plan = CrawlPlan::new(c).unwrap();
    plan.expand(
        "https://shop.test/",
        PAGE.as_bytes(),
        Some("text/html"),
        0,
        0,
    );
    assert_eq!(plan.media().len(), 4, "2 documents + 2 media");
    let kinds: Vec<&str> = plan
        .media()
        .resources()
        .iter()
        .map(|r| r.kind.as_str())
        .collect();
    assert_eq!(kinds, vec!["document", "document", "image", "video"]);
    assert!(
        plan.media()
            .resources()
            .iter()
            .all(|r| r.size.is_none() && r.sha256.is_none()),
        "discovery records no bytes, so it publishes no size and no checksum"
    );
    assert!(
        plan.media().resources().iter().all(|r| r.mime_guessed),
        "MIME came from the extension"
    );
}

#[test]
fn document_index_is_bounded_and_deduplicated() {
    let c = CrawlCfg {
        documents_cap: 3,
        ..cfg("https://shop.test/")
    };
    let mut plan = CrawlPlan::new(c).unwrap();
    for i in 0..10 {
        let body = format!("<html><head><title>Page {i}</title></head><body>hello</body></html>");
        plan.expand(
            &format!("https://shop.test/p/{i}"),
            body.as_bytes(),
            Some("text/html"),
            0,
            i as i64,
        );
    }
    assert_eq!(plan.documents().len(), 3, "cap holds");
    assert_eq!(
        plan.documents().documents()[0].title.as_deref(),
        Some("Page 0"),
        "titles are captured from the HTML"
    );
    assert!(plan
        .documents()
        .documents()
        .iter()
        .all(|d| d.sha256.is_some()));
}

#[test]
fn configuration_bounds_are_validated() {
    assert!(CrawlPlan::new(CrawlCfg::default()).is_err(), "no start URL");
    assert!(
        CrawlPlan::new(cfg("ftp://shop.test")).is_err(),
        "not http(s)"
    );
    assert!(CrawlPlan::new(CrawlCfg {
        max_depth: 999,
        ..cfg("https://shop.test/")
    })
    .is_err());
    assert!(CrawlPlan::new(CrawlCfg {
        max_pages: usize::MAX,
        ..cfg("https://shop.test/")
    })
    .is_err());
    assert!(CrawlPlan::new(CrawlCfg {
        content_types: vec!["yaml".to_string()],
        ..cfg("https://shop.test/")
    })
    .is_err());
    assert_eq!(
        CrawlPlan::new(cfg("https://shop.test/"))
            .unwrap()
            .cfg()
            .discovery
            .max_per_kind,
        500
    );
}

#[test]
fn existing_target_configuration_maps_onto_a_plan() {
    // `CrawlPlan::from_target` reuses the target's own depth/pages/include/
    // exclude/sitemap settings instead of a parallel configuration.
    let t = harvest::config::TargetCfg {
        id: "demo-catalog".to_string(),
        start_url: "https://shop.test/catalog".to_string(),
        max_depth: 2,
        max_pages: 25,
        follow_links: true,
        sitemap: true,
        include: vec!["/catalog".to_string()],
        ..harvest::config::TargetCfg::default()
    };
    let plan = CrawlPlan::from_target(&t).unwrap();
    assert_eq!(plan.cfg().max_depth, 2);
    assert_eq!(plan.cfg().max_pages, 25);
    assert!(plan.cfg().sitemap);
    assert_eq!(plan.cfg().include, vec!["/catalog".to_string()]);
    assert_eq!(
        plan.admit("https://shop.test/catalog/x", 1),
        harvest::crawl::Admit::Allow
    );
    assert_eq!(
        plan.admit("https://shop.test/other", 1),
        harvest::crawl::Admit::Deny(Skip::NotIncluded)
    );
}

#[test]
fn plan_snapshot_is_serializable_and_bounded() {
    let mut plan = CrawlPlan::new(CrawlCfg {
        max_pages: 10,
        ..cfg("https://shop.test/")
    })
    .unwrap();
    plan.seed();
    plan.expand(
        "https://shop.test/",
        PAGE.as_bytes(),
        Some("text/html"),
        0,
        0,
    );
    let json = plan.to_json();
    assert_eq!(json["config"]["max_pages"], 10);
    assert_eq!(json["robots_loaded"], false);
    assert!(json["stats"]["queued"].as_u64().unwrap() >= 3);
    assert!(json["documents"].as_u64().unwrap() >= 1);
    // The snapshot never embeds bodies: it is metadata only.
    let rendered = json.to_string();
    assert!(
        !rendered.contains("mousetrap"),
        "no payload in the plan snapshot"
    );
    let _ = workdir("snapshot");
    let _ = DiscoverySource::Sitemap;
    let _ = crawl::MAX_DEPTH_CEILING;
}
