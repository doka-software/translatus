//! A byte-faithful XHTML mini-DOM.
//!
//! We slice the *exact* source bytes for every event (start tags with their
//! attributes, end tags, text, comments, the XML declaration, the doctype…) and
//! re-emit them verbatim on serialize. Only the inner content of leaf blocks that
//! we translate is ever rewritten, so everything else round-trips unchanged —
//! which is what makes "preserve the layout" actually hold.

use crate::format::placeholder;
use std::collections::BTreeMap;

/// Inline elements: translatable text can flow through them, so they become
/// paired `⟦n⟧…⟦/n⟧` placeholders rather than block boundaries.
const INLINE: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "dfn", "em", "i", "kbd", "mark", "q",
    "rp", "rt", "ruby", "s", "samp", "small", "span", "strong", "sub", "sup", "time", "u", "var",
    "wbr", "br", "img", "wbr",
];

/// Block elements whose direct text we translate as one segment (when they are
/// "leaf" blocks — i.e. contain no nested block element).
// Note: `title` is intentionally excluded — it only appears in <head>, where a
// bilingual sibling would be invalid (one <title> per document).
const BLOCK: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "dd",
    "dt",
    "blockquote",
    "figcaption",
    "caption",
    "td",
    "th",
    "div",
    "label",
    "summary",
];

fn local_name(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

fn is_inline(name: &str) -> bool {
    INLINE.contains(&local_name(name).as_str())
}
fn is_block(name: &str) -> bool {
    BLOCK.contains(&local_name(name).as_str())
}

#[derive(Debug, Clone)]
pub enum Node {
    Elem {
        name: String,
        block: bool,
        inline: bool,
        self_closing: bool,
        raw_start: String,
        raw_end: String,
        children: Vec<Node>,
    },
    /// Text / comment / CDATA / decl / PI / doctype — stored as exact source bytes.
    Raw(String),
}

impl Node {
    fn is_block_elem(&self) -> bool {
        matches!(self, Node::Elem { block: true, .. })
    }
}

#[derive(Debug, Clone)]
pub struct Dom {
    pub roots: Vec<Node>,
}

/// Maximum element nesting depth. Parsing itself is iterative, but the later
/// walks (serialize / segment collection / apply) recurse per nesting level —
/// a malicious document with hundreds of thousands of nested elements would
/// overflow the stack. Real books nest well under 100 levels.
const MAX_DEPTH: usize = 256;

impl Dom {
    /// Parse XHTML source into the faithful DOM.
    ///
    /// Entity safety: quick-xml never resolves external entities (no XXE) and
    /// never expands custom DTD entities (no billion-laughs blowup) — unknown
    /// entity references simply round-trip as raw bytes.
    pub fn parse(src: &str) -> crate::error::Result<Dom> {
        use quick_xml::events::Event;
        use quick_xml::Reader;

        let mut reader = Reader::from_str(src);
        let cfg = reader.config_mut();
        cfg.trim_text(false);
        cfg.expand_empty_elements = false;
        cfg.check_end_names = false;

        // children-stack + open-element metadata stack
        let mut stack: Vec<Vec<Node>> = vec![Vec::new()];
        let mut open: Vec<(String, String)> = Vec::new(); // (name, raw_start)
        let mut last = 0usize;

        loop {
            let ev = reader.read_event();
            let pos = reader.buffer_position() as usize;
            let raw = src.get(last..pos).unwrap_or("").to_string();
            last = pos;
            match ev {
                Ok(Event::Eof) => break,
                Ok(Event::Start(e)) => {
                    if open.len() >= MAX_DEPTH {
                        return Err(crate::error::CoreError::MalformedEpub(format!(
                            "XHTML element nesting exceeds the depth limit ({MAX_DEPTH})"
                        )));
                    }
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    open.push((name, raw));
                    stack.push(Vec::new());
                }
                Ok(Event::End(_)) => {
                    let children = stack.pop().unwrap_or_default();
                    if let Some((name, raw_start)) = open.pop() {
                        let node = Node::Elem {
                            block: is_block(&name),
                            inline: is_inline(&name),
                            self_closing: false,
                            raw_start,
                            raw_end: raw,
                            name,
                            children,
                        };
                        stack.last_mut().unwrap().push(node);
                    } else {
                        stack.last_mut().unwrap().push(Node::Raw(raw));
                    }
                }
                Ok(Event::Empty(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    let node = Node::Elem {
                        block: is_block(&name),
                        inline: is_inline(&name),
                        self_closing: true,
                        raw_start: raw,
                        raw_end: String::new(),
                        name,
                        children: Vec::new(),
                    };
                    stack.last_mut().unwrap().push(node);
                }
                Ok(_) => {
                    stack.last_mut().unwrap().push(Node::Raw(raw));
                }
                Err(e) => return Err(e.into()),
            }
        }

        // Unwind any unclosed elements faithfully.
        while !open.is_empty() {
            let children = stack.pop().unwrap_or_default();
            let (name, raw_start) = open.pop().unwrap();
            let node = Node::Elem {
                block: is_block(&name),
                inline: is_inline(&name),
                self_closing: false,
                raw_start,
                raw_end: String::new(),
                name,
                children,
            };
            stack.last_mut().unwrap().push(node);
        }

        Ok(Dom {
            roots: stack.pop().unwrap_or_default(),
        })
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for n in &self.roots {
            write_node(n, &mut out);
        }
        out
    }

    /// Extract one segment per non-empty leaf block, in document order.
    pub fn extract_segments(&self) -> Vec<crate::document::Segment> {
        let mut out = Vec::new();
        let mut idx = 0usize;
        for n in &self.roots {
            collect(n, &mut idx, &mut out);
        }
        out
    }

    /// Write `target` translations and `notes` annotations back into the
    /// matching leaf blocks. A note's `pos` decides its side of the block
    /// (AN-014): Before = the sibling precedes the source block, After = it
    /// follows (after the translation sibling in bilingual mode). `lang`
    /// labels the translated nodes (bilingual mode); `note_lang` labels
    /// annotation blocks (None = no lang attribute).
    pub fn apply_segments(
        &mut self,
        targets: &std::collections::HashMap<usize, String>,
        notes: &std::collections::HashMap<usize, crate::document::Note>,
        mode: crate::config::OutputMode,
        lang: &str,
        note_lang: Option<&str>,
    ) {
        let mut idx = 0usize;
        let roots = std::mem::take(&mut self.roots);
        self.roots = apply_nodes(roots, &mut idx, targets, notes, mode, lang, note_lang);
    }
}

fn write_node(n: &Node, out: &mut String) {
    match n {
        Node::Raw(s) => out.push_str(s),
        Node::Elem {
            self_closing,
            raw_start,
            raw_end,
            children,
            ..
        } => {
            out.push_str(raw_start);
            if !self_closing {
                for c in children {
                    write_node(c, out);
                }
                out.push_str(raw_end);
            }
        }
    }
}

/// Does this element contain a nested block element among its descendants?
fn has_block_descendant(children: &[Node]) -> bool {
    for c in children {
        if let Node::Elem { children: gc, .. } = c {
            if c.is_block_elem() || has_block_descendant(gc) {
                return true;
            }
        }
    }
    false
}

/// Serialize a leaf block's inline content into placeholder text + map.
/// Returns None when there is no translatable (non-whitespace) text.
fn block_segment(children: &[Node]) -> Option<(String, BTreeMap<String, String>)> {
    let mut out = String::new();
    let mut map = BTreeMap::new();
    let mut counter = 1usize;
    serialize_inline(children, &mut out, &mut map, &mut counter);
    if out.trim().is_empty() {
        None
    } else {
        Some((out, map))
    }
}

fn serialize_inline(
    nodes: &[Node],
    out: &mut String,
    map: &mut BTreeMap<String, String>,
    counter: &mut usize,
) {
    for n in nodes {
        match n {
            Node::Raw(raw) => {
                let t = raw.trim_start();
                if t.starts_with("<!--") || t.starts_with("<![CDATA[") || t.starts_with("<?") {
                    let id = format!("C{}", *counter);
                    *counter += 1;
                    map.insert(id.clone(), raw.clone());
                    push_token(out, &id);
                } else {
                    // plain text — unescape so the LLM sees real characters
                    let un = quick_xml::escape::unescape(raw)
                        .map(|c| c.into_owned())
                        .unwrap_or_else(|_| raw.clone());
                    out.push_str(&un);
                }
            }
            Node::Elem {
                self_closing: true,
                raw_start,
                ..
            } => {
                let id = format!("C{}", *counter);
                *counter += 1;
                map.insert(id.clone(), raw_start.clone());
                push_token(out, &id);
            }
            Node::Elem {
                raw_start,
                raw_end,
                children,
                ..
            } => {
                let id = counter.to_string();
                *counter += 1;
                map.insert(id.clone(), raw_start.clone());
                map.insert(format!("/{}", id), raw_end.clone());
                push_token(out, &id);
                serialize_inline(children, out, map, counter);
                push_token(out, &format!("/{}", id));
            }
        }
    }
}

fn push_token(out: &mut String, id: &str) {
    out.push(placeholder::OPEN);
    out.push_str(id);
    out.push(placeholder::CLOSE);
}

fn collect(n: &Node, idx: &mut usize, out: &mut Vec<crate::document::Segment>) {
    if let Node::Elem {
        block, children, ..
    } = n
    {
        if *block && !has_block_descendant(children) {
            if let Some((source, map)) = block_segment(children) {
                out.push(crate::document::Segment::new(*idx, source, map));
                *idx += 1;
            }
            return;
        }
        for c in children {
            collect(c, idx, out);
        }
    }
}

/// A translated sibling: same tag as the source block, but a fresh element with
/// our `etc-trans` class + `lang`/`dir` and crucially **no `id`** (cloning the id
/// would create duplicate ids and break EPUB/TOC anchors — an actual bug in some
/// competitors). The source block is left byte-faithful; only this sibling is new.
fn translation_sibling(name: &str, inner_html: &str, lang: &str) -> String {
    let local = name.rsplit(':').next().unwrap_or(name);
    format!(
        "<{n} class=\"etc-trans\" lang=\"{l}\" dir=\"auto\">{c}</{n}>",
        n = local,
        l = esc_attr(lang),
        c = inner_html
    )
}

/// Text prefix marking a note as the annotator's voice, never the book's.
/// Deliberately a compact, language-neutral bracket form.
pub const NOTE_PREFIX: &str = "〔註〕";

/// An annotation sibling: a fresh `<div class="etc-note">` (no id — same
/// duplicate-id rationale as `translation_sibling`) placed before or after the
/// block it annotates per the note's `pos` (after the translation sibling in
/// bilingual mode when placed after). The 〔註〕 prefix keeps the note
/// self-identifying as a margin note in any reader, with or without our CSS.
fn annotation_sibling(inner_html: &str, note_lang: Option<&str>) -> String {
    let lang_attr = note_lang
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!(" lang=\"{}\"", esc_attr(l)))
        .unwrap_or_default();
    format!("<div class=\"etc-note\"{lang_attr} dir=\"auto\">{NOTE_PREFIX} {inner_html}</div>")
}

/// Escape a value destined for a double-quoted XML attribute. The target-language
/// label flows in from user input (`--to`), so an unescaped `"`/`&`/`<` would
/// break well-formedness or inject markup into the bilingual sibling.
fn esc_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Rebuild a node list, applying translations and annotations. Replace swaps
/// the block's inner content; Bilingual keeps the (byte-faithful) source block
/// and inserts a translated sibling right after it — "one paragraph source, one
/// paragraph translation". A segment's note (if any) is its own sibling on the
/// side its `pos` dictates (AN-014): Before = ahead of the source block
/// (background the reader needs first), After = after the block (after the
/// translation sibling when one exists).
#[allow(clippy::too_many_arguments)]
fn apply_nodes(
    nodes: Vec<Node>,
    idx: &mut usize,
    targets: &std::collections::HashMap<usize, String>,
    notes: &std::collections::HashMap<usize, crate::document::Note>,
    mode: crate::config::OutputMode,
    lang: &str,
    note_lang: Option<&str>,
) -> Vec<Node> {
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node {
            Node::Elem {
                name,
                block,
                inline,
                self_closing,
                raw_start,
                raw_end,
                children,
            } => {
                let is_leaf_block = block && !has_block_descendant(&children);
                if is_leaf_block && block_segment(&children).is_some() {
                    let i = *idx;
                    *idx += 1;
                    let note = notes.get(&i).filter(|n| !n.text.trim().is_empty());
                    if let Some(n) = note.filter(|n| n.pos == crate::document::NotePos::Before) {
                        out.push(Node::Raw(annotation_sibling(&n.text, note_lang)));
                        out.push(Node::Raw("\n".to_string()));
                    }
                    match (mode, targets.get(&i)) {
                        (crate::config::OutputMode::Replace, Some(t)) => {
                            out.push(Node::Elem {
                                name,
                                block,
                                inline,
                                self_closing,
                                raw_start,
                                raw_end,
                                children: vec![Node::Raw(t.clone())],
                            });
                        }
                        (crate::config::OutputMode::Bilingual, Some(t)) => {
                            let trans = translation_sibling(&name, t, lang);
                            out.push(Node::Elem {
                                name,
                                block,
                                inline,
                                self_closing,
                                raw_start,
                                raw_end,
                                children,
                            });
                            out.push(Node::Raw("\n".to_string()));
                            out.push(Node::Raw(trans));
                        }
                        (_, None) => {
                            out.push(Node::Elem {
                                name,
                                block,
                                inline,
                                self_closing,
                                raw_start,
                                raw_end,
                                children,
                            });
                        }
                    }
                    if let Some(n) = note.filter(|n| n.pos == crate::document::NotePos::After) {
                        out.push(Node::Raw("\n".to_string()));
                        out.push(Node::Raw(annotation_sibling(&n.text, note_lang)));
                    }
                } else {
                    let new_children =
                        apply_nodes(children, idx, targets, notes, mode, lang, note_lang);
                    out.push(Node::Elem {
                        name,
                        block,
                        inline,
                        self_closing,
                        raw_start,
                        raw_end,
                        children: new_children,
                    });
                }
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- malicious-input guards (audit 2026-07: XXE / entity bombs / depth) ----

    #[test]
    fn billion_laughs_entities_are_not_expanded() {
        // Classic entity-expansion bomb. quick-xml must not expand custom DTD
        // entities: the doctype and every `&lolN;` reference round-trip as raw
        // bytes, so output size stays O(input) instead of exploding.
        let src = r#"<?xml version="1.0"?>
<!DOCTYPE lolz [
 <!ENTITY lol "lol">
 <!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
 <!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">
 <!ENTITY lol4 "&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;&lol3;">
]>
<html><body><p>&lol4;&lol4;&lol4;</p></body></html>"#;
        let dom = Dom::parse(src).unwrap();
        let out = dom.serialize();
        assert_eq!(out, src, "entity bomb must round-trip unexpanded");
        // The segment the LLM would see keeps the reference inert as well.
        let segs = dom.extract_segments();
        assert_eq!(segs.len(), 1);
        assert!(segs[0].source.contains("&lol4;") || segs[0].source.contains("lol4"));
        assert!(segs[0].source.len() < 200, "no expansion in segment text");
    }

    #[test]
    fn external_entities_are_never_resolved() {
        // XXE probe: a SYSTEM entity pointing at a local file must never be
        // fetched or substituted.
        let src = r#"<?xml version="1.0"?>
<!DOCTYPE r [ <!ENTITY xxe SYSTEM "file:///etc/passwd"> ]>
<html><body><p>&xxe;</p></body></html>"#;
        let dom = Dom::parse(src).unwrap();
        let out = dom.serialize();
        assert_eq!(out, src, "external entity must round-trip unresolved");
        assert!(!out.contains("root:"), "file content must never appear");
    }

    #[test]
    fn nesting_deeper_than_cap_is_rejected() {
        // 300 nested <div> (over MAX_DEPTH=256): parse must fail cleanly
        // instead of building a tree whose recursive walks blow the stack.
        let mut src = String::from("<html><body>");
        for _ in 0..300 {
            src.push_str("<div>");
        }
        src.push('x');
        for _ in 0..300 {
            src.push_str("</div>");
        }
        src.push_str("</body></html>");
        let err = Dom::parse(&src).unwrap_err();
        assert!(err.to_string().contains("depth limit"));
    }

    #[test]
    fn nesting_within_cap_still_parses() {
        let mut src = String::from("<html><body>");
        for _ in 0..100 {
            src.push_str("<div>");
        }
        src.push_str("<p>deep</p>");
        for _ in 0..100 {
            src.push_str("</div>");
        }
        src.push_str("</body></html>");
        let dom = Dom::parse(&src).unwrap();
        assert_eq!(dom.serialize(), src);
        assert_eq!(dom.extract_segments().len(), 1);
    }

    #[test]
    fn roundtrip_untouched() {
        let src = r#"<?xml version="1.0"?><html><body><p>Hello <b>world</b>.</p><div><p>x</p></div></body></html>"#;
        let dom = Dom::parse(src).unwrap();
        assert_eq!(dom.serialize(), src);
    }

    #[test]
    fn extract_leaf_blocks() {
        let src =
            r#"<html><body><p>Hello <b>world</b>.</p><div>wrap<p>inner</p></div></body></html>"#;
        let dom = Dom::parse(src).unwrap();
        let segs = dom.extract_segments();
        // two <p> leaf blocks; the wrapping <div> has a block descendant so is not a leaf
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].source, "Hello ⟦1⟧world⟦/1⟧.");
        assert_eq!(segs[1].source, "inner");
    }

    #[test]
    fn apply_replace() {
        let src = r#"<html><body><p>Hello <b>world</b>.</p></body></html>"#;
        let mut dom = Dom::parse(src).unwrap();
        let segs = dom.extract_segments();
        let inner = placeholder::restore("你好⟦1⟧世界⟦/1⟧。", &segs[0].placeholders);
        let mut targets = std::collections::HashMap::new();
        targets.insert(0usize, inner);
        let notes = std::collections::HashMap::new();
        dom.apply_segments(
            &targets,
            &notes,
            crate::config::OutputMode::Replace,
            "zh-Hant",
            None,
        );
        assert_eq!(
            dom.serialize(),
            r#"<html><body><p>你好<b>世界</b>。</p></body></html>"#
        );
    }

    #[test]
    fn apply_bilingual_inserts_sibling() {
        // "one paragraph source, one paragraph translation": source <p> stays
        // byte-faithful; a translated sibling <p class="etc-trans"> follows it.
        let src = r#"<html><body><p id="a">Hello.</p></body></html>"#;
        let mut dom = Dom::parse(src).unwrap();
        let mut targets = std::collections::HashMap::new();
        targets.insert(0usize, "你好。".to_string());
        let notes = std::collections::HashMap::new();
        dom.apply_segments(
            &targets,
            &notes,
            crate::config::OutputMode::Bilingual,
            "zh-Hant",
            None,
        );
        let out = dom.serialize();
        // original preserved verbatim (incl. its id)
        assert!(out.contains(r#"<p id="a">Hello.</p>"#));
        // translated sibling: same tag, our class, lang/dir, and NO id
        assert!(out.contains(r#"<p class="etc-trans" lang="zh-Hant" dir="auto">你好。</p>"#));
        assert!(out.find("Hello.").unwrap() < out.find("你好。").unwrap());
    }

    #[test]
    fn apply_note_without_translation_keeps_source_byte_faithful() {
        // annotate-only: source <p> untouched, after-note sibling follows it.
        let src = r#"<html><body><p id="a">Hello.</p><p>Plain.</p></body></html>"#;
        let mut dom = Dom::parse(src).unwrap();
        let targets = std::collections::HashMap::new();
        let mut notes = std::collections::HashMap::new();
        notes.insert(0usize, crate::document::Note::after("背景說明。"));
        dom.apply_segments(
            &targets,
            &notes,
            crate::config::OutputMode::Replace,
            "zh-Hant",
            Some("zh-Hant"),
        );
        let out = dom.serialize();
        assert!(
            out.contains(r#"<p id="a">Hello.</p>"#),
            "source byte-faithful"
        );
        assert!(out.contains(r#"<p>Plain.</p>"#), "unnoted block untouched");
        assert!(out.contains(
            r#"<div class="etc-note" lang="zh-Hant" dir="auto">〔註〕 背景說明。</div>"#
        ));
        assert!(out.find("Hello.").unwrap() < out.find("背景說明").unwrap());
    }

    #[test]
    fn apply_before_note_precedes_source_block() {
        // AN-014: a before-note (背景鋪墊) renders AHEAD of the block it
        // annotates, source still byte-faithful.
        let src = r#"<html><body><p>Intro.</p><p id="a">Hello.</p></body></html>"#;
        let mut dom = Dom::parse(src).unwrap();
        let targets = std::collections::HashMap::new();
        let mut notes = std::collections::HashMap::new();
        notes.insert(
            1usize,
            crate::document::Note::new(crate::document::NotePos::Before, "先備背景。"),
        );
        dom.apply_segments(
            &targets,
            &notes,
            crate::config::OutputMode::Replace,
            "zh-Hant",
            Some("zh-Hant"),
        );
        let out = dom.serialize();
        assert!(out.contains(r#"<p id="a">Hello.</p>"#), "byte-faithful");
        let intro = out.find("Intro.").unwrap();
        let note = out.find("先備背景").unwrap();
        let hello = out.find("Hello.").unwrap();
        assert!(
            intro < note && note < hello,
            "before-note sits between the previous block and its own block"
        );
    }

    #[test]
    fn apply_before_note_precedes_source_in_bilingual() {
        let src = r#"<html><body><p>Hello.</p></body></html>"#;
        let mut dom = Dom::parse(src).unwrap();
        let mut targets = std::collections::HashMap::new();
        targets.insert(0usize, "你好。".to_string());
        let mut notes = std::collections::HashMap::new();
        notes.insert(
            0usize,
            crate::document::Note::new(crate::document::NotePos::Before, "問候語的由來。"),
        );
        dom.apply_segments(
            &targets,
            &notes,
            crate::config::OutputMode::Bilingual,
            "zh-Hant",
            Some("zh-Hant"),
        );
        let out = dom.serialize();
        let note_pos = out.find("etc-note").unwrap();
        let src_pos = out.find("Hello.").unwrap();
        let trans_pos = out.find("etc-trans").unwrap();
        assert!(
            note_pos < src_pos && src_pos < trans_pos,
            "order: 眉批(before)→原文→譯文"
        );
    }

    #[test]
    fn apply_note_follows_translation_sibling_in_bilingual() {
        let src = r#"<html><body><p>Hello.</p></body></html>"#;
        let mut dom = Dom::parse(src).unwrap();
        let mut targets = std::collections::HashMap::new();
        targets.insert(0usize, "你好。".to_string());
        let mut notes = std::collections::HashMap::new();
        notes.insert(0usize, crate::document::Note::after("問候語的由來。"));
        dom.apply_segments(
            &targets,
            &notes,
            crate::config::OutputMode::Bilingual,
            "zh-Hant",
            Some("zh-Hant"),
        );
        let out = dom.serialize();
        let src_pos = out.find("Hello.").unwrap();
        let trans_pos = out.find("etc-trans").unwrap();
        let note_pos = out.find("etc-note").unwrap();
        assert!(
            src_pos < trans_pos && trans_pos < note_pos,
            "order: 原文→譯文→眉批(after)"
        );
        // the note carries no id and never impersonates the source tag
        assert!(out.contains(r#"<div class="etc-note""#));
    }
}
