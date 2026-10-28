use super::*;

#[test]
fn disallow_prefix_with_allow_override() {
    let r = Robots::parse(
        "User-agent: *\nDisallow: /private/\nAllow: /private/public.html\n",
        "harvest",
    );
    assert!(!r.allows("/private/secret"));
    assert!(r.allows("/private/public.html"));
    assert!(r.allows("/public"));
}

#[test]
fn full_site_disallow() {
    let r = Robots::parse("User-agent: *\nDisallow: /\n", "harvest");
    assert!(r.full_disallow);
    assert!(!r.allows("/anything"));
}

#[test]
fn empty_disallow_allows_all() {
    let r = Robots::parse("User-agent: *\nDisallow:\n", "harvest");
    assert!(!r.full_disallow);
    assert!(r.allows("/x"));
}

#[test]
fn ua_specific_over_star() {
    let txt = "User-agent: *\nDisallow: /a/\n\nUser-agent: harvest\nDisallow: /b/\n";
    let r = Robots::parse(txt, "harvest");
    assert!(
        r.allows("/a/"),
        "harvest group should override star: {:?}",
        r.rules
    );
    assert!(!r.allows("/b/"));
}

#[test]
fn crawl_delay_and_sitemap() {
    let r = Robots::parse(
        "User-agent: *\nDisallow:\nCrawl-delay: 2.5\nSitemap: https://x/sitemap.xml\n",
        "harvest",
    );
    assert_eq!(r.crawl_delay, Some(Duration::from_millis(2500)));
    assert_eq!(r.sitemap.as_deref(), Some("https://x/sitemap.xml"));
}

#[test]
fn longest_prefix_wins() {
    let r = Robots::parse(
        "User-agent: *\nDisallow: /docs/\nAllow: /docs/open/\n",
        "harvest",
    );
    assert!(!r.allows("/docs/closed"));
    assert!(r.allows("/docs/open/page"));
}

#[test]
fn audit_invalid_crawl_delays_never_panic() {
    for value in ["-1", "NaN", "inf", "1e300"] {
        let r = Robots::parse(&format!("User-agent: *\nCrawl-delay: {value}\n"), "harvest");
        assert_eq!(r.crawl_delay, None, "invalid delay {value}");
    }
}

#[test]
fn audit_allow_wins_equal_length_and_ua_matching_is_not_reversed() {
    let r = Robots::parse("User-agent: *\nDisallow: /same\nAllow: /same\n", "harvest");
    assert!(r.allows("/same"));
    let text = "User-agent: harvest\nDisallow: /private\nUser-agent: *\nAllow: /\n";
    assert!(!Robots::parse(text, "harvest-demo").allows("/private"));
    let text = "User-agent: harvest-special\nDisallow: /\nUser-agent: *\nAllow: /\n";
    assert!(Robots::parse(text, "harvest").allows("/public"));
}

#[test]
fn audit_robot_wildcards_end_anchors_and_delay_floor() {
    let r = Robots::parse("User-agent: *\nDisallow: /*.pdf$\nDisallow: /search?secret=\nCrawl-delay: 1\nCrawl-delay: 3\n", "harvest");
    assert!(!r.allows("/docs/report.pdf"));
    assert!(r.allows("/docs/report.pdf.html"));
    assert!(!r.allows("/search?secret=yes"));
    assert_eq!(r.crawl_delay, Some(Duration::from_secs(3)));
}
