//! Minimal, *lenient* sitemap.xml reader (sitemaps.org schema / RFC 9309 §6).
//!
//! We only need `<loc>` text between `<url>`/`<sitemap>` tags, so a scanner
//! beats a full XML tree: it survives the sloppy-but-live sitemaps real
//! sites publish (missing namespaces, uppercase tags, CDATA-wrapped URLs,
//! bare `&#38;` entities) while a strict parser rejects the whole file.
//! Anything unparseable degrades to an empty list — a run must never die
//! just because a marketing CMS emitted a stray tag.

/// Cap work per file even on a hostile/garbage payload.
const MAX_LOCS: usize = 50_000;

#[derive(Debug, PartialEq)]
pub enum Sitemap {
    /// Flat `<urlset>`: collectable page URLs.
    Urls(Vec<String>),
    /// `<sitemapindex>`: child sitemap documents to fetch next.
    Index(Vec<String>),
}

pub fn parse(xml: &str) -> Sitemap {
    let is_index = xml.to_ascii_lowercase().contains("<sitemapindex");
    let mut locs = Vec::new();
    let mut from = 0usize;
    while locs.len() < MAX_LOCS {
        let Some(open) = find_ci(xml, from, "<loc>") else {
            break;
        };
        let start = open + 5; // len("<loc>") — lowercase match keeps byte length
        let Some(close) = find_ci(xml, start, "</loc>") else {
            break;
        };
        let raw = &xml[start..close];
        from = close + 6;
        let txt = clean_loc(raw);
        // Accept absolute URLs and root-relative locs (the caller resolves
        // the latter against the document URL); reject junk.
        let plausible = txt.contains("://") || txt.starts_with('/');
        if !txt.is_empty() && !txt.contains(char::is_whitespace) && plausible {
            locs.push(txt);
        }
    }
    if is_index {
        Sitemap::Index(locs)
    } else {
        Sitemap::Urls(locs)
    }
}

/// Strip CDATA wrapper + decode the handful of XML entities URLs actually use.
fn clean_loc(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("<![CDATA[") {
        s = rest.strip_suffix("]]>").unwrap_or(rest).trim();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.char_indices().peekable();
    while let Some((_, c)) = it.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        // collect until ';'
        let mut ent = String::new();
        let mut consumed = true;
        for (i, ch) in it.by_ref() {
            if ch == ';' {
                let _ = i;
                break;
            }
            if !(ch.is_ascii_alphanumeric() || ch == '#') {
                consumed = false;
                break;
            }
            ent.push(ch);
        }
        if !consumed || ent.is_empty() {
            out.push('&');
            out.push_str(&ent);
            continue;
        }
        match ent.as_str() {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                match u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push('&');
                        out.push_str(&ent);
                        out.push(';');
                    }
                }
            }
            _ if ent.starts_with('#') => {
                match ent[1..].parse::<u32>().ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push('&');
                        out.push_str(&ent);
                        out.push(';');
                    }
                }
            }
            _ => {
                out.push('&');
                out.push_str(&ent);
                out.push(';');
            }
        }
    }
    out.trim().to_string()
}

fn find_ci(hay: &str, from: usize, needle: &str) -> Option<usize> {
    hay[from..]
        .to_ascii_lowercase()
        .find(needle)
        .map(|i| i + from)
}

#[cfg(test)]
mod tests;
