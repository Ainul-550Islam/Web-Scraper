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
