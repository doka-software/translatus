//! Which spine documents are publisher apparatus rather than the book.
//!
//! A copyright page is the one page in a book that must not be rewritten: it
//! is the publisher's legal notice, its ISBN and cataloguing data, and its
//! rights statement. Translating it produces an unauthorised-looking restatement
//! of exactly the text that says the work may not be restated, costs tokens,
//! and gives the reader nothing. Japanese books put the same thing (奥付) at
//! the back rather than the front, so "front matter" is about role, not
//! position.

use std::collections::HashSet;

/// `epub:type` / EPUB2 `guide` values that mark publisher apparatus.
const APPARATUS_TYPES: [&str; 3] = ["copyright-page", "imprint", "colophon"];

/// Hrefs (as they appear in the OPF, before base resolution) that the book
/// itself declares as apparatus.
pub fn declared(opf: &str, nav_docs: &[(String, String)]) -> HashSet<String> {
    let mut out = HashSet::new();
    // EPUB 2: <guide><reference type="copyright-page" href="..."/>
    for tag in tags_named(opf, "reference") {
        let (Some(t), Some(h)) = (attr(&tag, "type"), attr(&tag, "href")) else {
            continue;
        };
        if APPARATUS_TYPES.iter().any(|a| t.eq_ignore_ascii_case(a)) {
            out.insert(strip_fragment(h));
        }
    }
    // EPUB 3: <nav epub:type="landmarks"><a epub:type="copyright-page" href="…">
    for (_, doc) in nav_docs {
        for tag in tags_named(doc, "a") {
            let Some(h) = attr(&tag, "href") else {
                continue;
            };
            // The attribute is `epub:type`, but a prefix is arbitrary; match the
            // local name so a differently-bound namespace still resolves.
            let t = attr(&tag, "epub:type").or_else(|| attr(&tag, "type"));
            if let Some(t) = t {
                if t.split_whitespace()
                    .any(|w| APPARATUS_TYPES.iter().any(|a| w.eq_ignore_ascii_case(a)))
                {
                    out.insert(strip_fragment(h));
                }
            }
        }
    }
    out
}

/// Marker phrases that only appear together on a rights/colophon page.
const MARKERS: [&str; 14] = [
    "all rights reserved",
    "library of congress",
    "cataloging-in-publication",
    "isbn",
    "no part of this book",
    "penguin random house",
    "無断複製",
    "無断転載",
    "発行所",
    "発行者",
    "著作権法",
    "版權所有",
    "翻印必究",
    "著作權",
];

/// Fallback for books that declare nothing: a short document carrying several
/// rights markers at once.
///
/// Deliberately conservative on both axes. The length cap keeps it away from
/// real prose that happens to discuss copyright (this book's own chapters do),
/// and two independent markers keep a single stray "ISBN" from disqualifying a
/// page. A false positive leaves one page in its original language, which is
/// the safe direction; a false negative is only today's behaviour.
pub fn looks_like_apparatus(plain_text: &str, segment_count: usize) -> bool {
    const MAX_SEGMENTS: usize = 40;
    if segment_count > MAX_SEGMENTS {
        return false;
    }
    let hay = plain_text.to_lowercase();
    let hits = MARKERS.iter().filter(|m| hay.contains(*m)).count();
    hits >= 2
}

fn strip_fragment(h: &str) -> String {
    h.split('#').next().unwrap_or(h).to_string()
}

/// Every start tag with this local name, prefix-insensitive.
fn tags_named(hay: &str, local: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = hay;
    while let Some(at) = rest.find('<') {
        rest = &rest[at + 1..];
        let after_prefix = match rest.find([':', ' ', '\t', '\r', '\n', '>', '/']) {
            Some(i) if rest.as_bytes()[i] == b':' => &rest[i + 1..],
            _ => rest,
        };
        if let Some(tail) = after_prefix.strip_prefix(local) {
            if tail.starts_with([' ', '\t', '\r', '\n', '>', '/']) {
                if let Some(end) = after_prefix.find('>') {
                    out.push(after_prefix[..end].to_string());
                }
            }
        }
    }
    out
}

fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("{key}=\"");
    let start = tag.find(&pat)? + pat.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_copyright_page_is_found_in_either_epub_generation() {
        let guide = r#"<guide>
            <reference href="xhtml/001_cvi_Cover.xhtml" type="cover"/>
            <reference href="xhtml/003_crt_Copyright.xhtml" type="copyright-page"/>
        </guide>"#;
        let got = declared(guide, &[]);
        assert!(got.contains("xhtml/003_crt_Copyright.xhtml"));
        assert_eq!(got.len(), 1, "only apparatus, not the cover");

        let nav = r#"<nav epub:type="landmarks"><ol>
            <li><a href="xhtml/002_tit.xhtml" epub:type="titlepage">Title</a></li>
            <li><a href="xhtml/003_crt.xhtml#top" epub:type="copyright-page">Copyright</a></li>
        </ol></nav>"#;
        let got = declared("", &[("nav.xhtml".into(), nav.into())]);
        assert!(
            got.contains("xhtml/003_crt.xhtml"),
            "fragment must be dropped"
        );
        assert_eq!(got.len(), 1);
    }

    /// Japanese books usually declare nothing and put the 奥付 at the back.
    #[test]
    fn an_undeclared_colophon_is_recognised_by_its_markers() {
        let okuduke = "小学館ｅＢｏｏｋｓ 発行者 ○○ 発行所 株式会社小学館 \
                       本書の無断複製は著作権法上の例外を除き禁じられています ISBN978-4-09-000000-0";
        assert!(looks_like_apparatus(okuduke, 6));

        let western = "Copyright © 2025 by Morgan Housel. All rights reserved. \
                       Library of Congress Cataloging-in-Publication Data. ISBN 9780593716632";
        assert!(looks_like_apparatus(western, 12));
    }

    /// A chapter that discusses copyright is still a chapter.
    #[test]
    fn real_prose_is_not_mistaken_for_apparatus() {
        let prose = "Penguin Random House values and supports copyright. Copyright fuels \
                     creativity. Thank you for buying an authorized edition of this book and \
                     for complying with copyright laws by not reproducing, scanning, or \
                     distributing any part of it in any form without permission.";
        // Long documents are out of scope regardless of what they mention.
        assert!(!looks_like_apparatus(prose, 200));
        // And one marker alone is not enough.
        assert!(!looks_like_apparatus("The ISBN was printed crookedly.", 3));
    }
}
