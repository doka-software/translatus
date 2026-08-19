//! EPUB container: read the ZIP, resolve the OPF (manifest + spine), parse each
//! XHTML spine document into a faithful DOM, and write a clean EPUB back out with
//! only the translated text changed.

use crate::config::OutputMode;
use crate::document::{Book, Chapter, Format, Segment};
use crate::error::{CoreError, Result};
use crate::format::dom::Dom;
use crate::format::placeholder;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

/// Everything needed to re-emit the book after translation.
pub struct EpubDoc {
    /// Original entries in their original order (name, bytes).
    entries: Vec<(String, Vec<u8>)>,
    /// Per spine XHTML document: (zip path, parsed DOM).
    docs: Vec<(String, Dom)>,
}

/// Decompression guards. An EPUB is attacker-supplied input (books come from
/// anywhere), and we hold every decompressed entry in memory — so a zip bomb
/// (a few KB that inflate to many GB) must be rejected, not OOM the process.
/// Real books sit far below these ceilings.
pub(crate) struct Limits {
    /// Maximum size of the archive file itself, checked before it is read.
    ///
    /// The caps below all describe *decompressed* bytes, and none of them can
    /// fire until the file is already in memory — so without this, a 4 GB file
    /// renamed to `.epub` is a memory spike before the parser has an opinion
    /// about it. It does not even have to be a valid zip.
    pub max_file_bytes: u64,
    /// Maximum number of archive entries.
    pub max_entries: usize,
    /// Maximum decompressed size of a single entry.
    pub max_entry_bytes: u64,
    /// Maximum total decompressed size across all entries.
    pub max_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // Comfortably above any real book: Project Gutenberg's largest
            // illustrated EPUBs are tens of MB.
            max_file_bytes: 512 * 1024 * 1024, // 512 MiB
            max_entries: 65_536,
            max_entry_bytes: 256 * 1024 * 1024,  // 256 MiB
            max_total_bytes: 1024 * 1024 * 1024, // 1 GiB
        }
    }
}

/// Whether an archive entry name is a legal, containable EPUB path.
///
/// Reading is already safe — nothing is extracted to disk — but the writer
/// rebuilds the archive from these names, so a hostile one would ride into the
/// *output* file unchanged. That makes a translated book a zip-slip payload
/// carried by a tool the reader trusts, and the next program to unpack it
/// (Calibre, a sync script, a reader app) is the one that gets hit.
///
/// OCF requires relative paths with no `..` component, so nothing legitimate
/// is lost by refusing these.
fn is_containable_entry(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // Absolute, in either separator style, plus Windows drive letters.
    if name.starts_with('/') || name.starts_with('\\') {
        return false;
    }
    if name.len() >= 2 && name.as_bytes()[1] == b':' {
        return false;
    }
    // Treat `\` as a separator too: some unpackers do, so `a\..\..\x` is a
    // traversal on those even though a `/`-only split would not see it.
    !name
        .split(['/', '\\'])
        .any(|component| component == ".." || component == ".")
}

/// Read one entry with the decompression caps enforced. `total` accumulates
/// across calls so a many-small-entries bomb is bounded too. Returns
/// `Ok(None)` when the entry does not exist.
fn read_entry(
    archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>,
    name: &str,
    limits: &Limits,
    total: &mut u64,
) -> Result<Option<Vec<u8>>> {
    let Ok(f) = archive.by_name(name) else {
        return Ok(None);
    };
    // take(cap + 1): never buffer more than one byte past the cap, no matter
    // what the zip header claims the size is.
    let mut buf = Vec::new();
    f.take(limits.max_entry_bytes + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > limits.max_entry_bytes {
        return Err(CoreError::MalformedEpub(format!(
            "entry {name} exceeds the per-entry decompressed size limit ({} bytes)",
            limits.max_entry_bytes
        )));
    }
    *total = total.saturating_add(buf.len() as u64);
    if *total > limits.max_total_bytes {
        return Err(CoreError::MalformedEpub(format!(
            "archive exceeds the total decompressed size limit ({} bytes)",
            limits.max_total_bytes
        )));
    }
    Ok(Some(buf))
}

/// Naive single-attribute extractor for the small, well-formed OPF/container XML
/// (avoids pulling structure we don't need).
fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("{}=\"", key);
    let start = tag.find(&pat)? + pat.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn opf_path(container_xml: &str) -> Option<String> {
    // <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
    // Match the attribute directly so we don't trip over the parent <rootfiles>.
    attr(container_xml, "full-path").map(|s| s.to_string())
}

fn dir_of(path: &str) -> String {
    match path.rfind('/') {
        Some(i) => path[..=i].to_string(),
        None => String::new(),
    }
}

/// Find the next start tag with this local name, ignoring any namespace prefix.
///
/// `<item `, `<opf:item ` and `<ns0:item ` are the same element: the prefix is
/// arbitrary and only the namespace it binds to carries meaning. Real EPUBs
/// ship all three (Calibre and several Python toolchains emit prefixed OPFs),
/// and a literal `find("<item ")` reads those books as zero chapters.
/// Returns the offset of the opening `<`.
fn find_start_tag(hay: &str, local: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find('<') {
        let at = from + rel;
        let rest = &hay[at + 1..];
        // An XML prefix is an NCName: no whitespace, ends at the ':'.
        let after_prefix = match rest.find([':', ' ', '\t', '\r', '\n', '>', '/']) {
            Some(i) if rest.as_bytes()[i] == b':' => &rest[i + 1..],
            _ => rest,
        };
        if let Some(tail) = after_prefix.strip_prefix(local) {
            if tail.starts_with([' ', '\t', '\r', '\n', '>', '/']) {
                return Some(at);
            }
        }
        from = at + 1;
    }
    None
}

/// Parse the OPF: returns (manifest id->href, spine idref order, title).
fn parse_opf(opf: &str) -> (HashMap<String, String>, Vec<String>, Option<String>) {
    let mut manifest = HashMap::new();
    // <item id="x" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
    let mut search = opf;
    while let Some(i) = find_start_tag(search, "item") {
        search = &search[i..];
        if let Some(end) = search.find('>') {
            let tag = &search[..end];
            if let (Some(id), Some(href)) = (attr(tag, "id"), attr(tag, "href")) {
                manifest.insert(id.to_string(), href.to_string());
            }
            search = &search[end..];
        } else {
            break;
        }
    }

    let mut spine = Vec::new();
    let mut s = opf;
    while let Some(i) = find_start_tag(s, "itemref") {
        s = &s[i..];
        if let Some(end) = s.find('>') {
            let tag = &s[..end];
            if let Some(idref) = attr(tag, "idref") {
                spine.push(idref.to_string());
            }
            s = &s[end..];
        } else {
            break;
        }
    }

    let title = find_start_tag(opf, "title").and_then(|i| {
        let rest = &opf[i..];
        let open = rest.find('>')? + 1;
        let close = rest[open..].find("</")?;
        Some(rest[open..open + close].trim().to_string())
    });

    (manifest, spine, title)
}

fn is_xhtml(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".xhtml") || p.ends_with(".html") || p.ends_with(".htm")
}

/// Read an EPUB into translation IR + a reassembly handle.
pub fn extract(path: &Path) -> Result<(Book, EpubDoc)> {
    extract_with_limits(path, &Limits::default())
}

/// `extract` with explicit decompression limits (separated so tests can prove
/// the guards fire without building multi-hundred-MB fixtures).
///
/// Note on zip-slip / symlinks: entries are only ever held as in-memory
/// `(name, bytes)` pairs — nothing is extracted to the filesystem, so an
/// entry named `../../x` or a symlink entry cannot escape anywhere.
pub(crate) fn extract_with_limits(path: &Path, limits: &Limits) -> Result<(Book, EpubDoc)> {
    // Before `read`, not after: this is the only guard that can bound the very
    // first allocation.
    let file_size = std::fs::metadata(path)?.len();
    if file_size > limits.max_file_bytes {
        return Err(CoreError::MalformedEpub(format!(
            "file is {file_size} bytes, above the {} byte limit — refusing to read it into memory",
            limits.max_file_bytes
        )));
    }
    let bytes = std::fs::read(path)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;

    if archive.len() > limits.max_entries {
        return Err(CoreError::MalformedEpub(format!(
            "archive has {} entries (limit {})",
            archive.len(),
            limits.max_entries
        )));
    }

    // Snapshot every entry in order (we rewrite the whole archive on save).
    let mut total = 0u64;
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    {
        // Dropped here rather than at write time: one choke point, and it
        // keeps the hostile name out of the IR entirely, so no later code path
        // can reintroduce it. A book carrying these still translates — the
        // entries are not reachable content — it just cannot launder them.
        let names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
            .filter(|n| is_containable_entry(n))
            .collect();
        for name in names {
            if let Some(b) = read_entry(&mut archive, &name, limits, &mut total)? {
                entries.push((name, b));
            }
        }
    }

    let container = read_entry(&mut archive, "META-INF/container.xml", limits, &mut total)?
        .ok_or_else(|| CoreError::MalformedEpub("missing META-INF/container.xml".into()))?;
    let container = String::from_utf8_lossy(&container).to_string();
    let opf_rel = opf_path(&container)
        .ok_or_else(|| CoreError::MalformedEpub("cannot locate OPF in container.xml".into()))?;

    let opf_bytes = read_entry(&mut archive, &opf_rel, limits, &mut total)?
        .ok_or_else(|| CoreError::MalformedEpub(format!("missing OPF: {opf_rel}")))?;
    let opf = String::from_utf8_lossy(&opf_bytes).to_string();
    let (manifest, spine, title) = parse_opf(&opf);
    let base = dir_of(&opf_rel);

    let mut chapters = Vec::new();
    let mut docs = Vec::new();
    let mut spine_index = 0usize;
    // Index entries by name once: the spine loop below would otherwise be
    // O(entries × spine) — quadratic on a book where most entries are chapters.
    let entry_index: std::collections::HashMap<&str, &Vec<u8>> =
        entries.iter().map(|(n, b)| (n.as_str(), b)).collect();

    for idref in &spine {
        let Some(href) = manifest.get(idref) else {
            continue;
        };
        let full = format!("{base}{href}");
        if !is_xhtml(&full) {
            continue;
        }
        let Some(raw) = entry_index.get(full.as_str()).map(|b| (*b).clone()) else {
            continue;
        };
        let src = String::from_utf8_lossy(&raw).to_string();
        let dom = Dom::parse(&src)?;
        let segments: Vec<Segment> = dom.extract_segments();
        if segments.is_empty() {
            // still register the doc so write() emits it unchanged
            docs.push((full.clone(), dom));
            continue;
        }
        chapters.push(Chapter {
            spine_index,
            href: full.clone(),
            title: None,
            segments,
        });
        docs.push((full, dom));
        spine_index += 1;
    }

    // A book with nothing to translate is a parse failure, not a free run.
    // Silently returning an empty Book made an unreadable OPF look like a
    // finished estimate: "0 chapters, $0.00, completed" — the one output a
    // user cannot tell apart from "this book is already done".
    if chapters.is_empty() {
        return Err(CoreError::MalformedEpub(format!(
            "no translatable chapters found in {opf_rel} \
             (manifest items: {}, spine entries: {}) — the OPF spine is empty \
             or its documents could not be read",
            manifest.len(),
            spine.len()
        )));
    }

    let book = Book {
        format: Format::Epub,
        chapters,
        title,
    };
    Ok((book, EpubDoc { entries, docs }))
}

/// CSS for the translated sibling in bilingual mode. An accent left-border marks
/// the translation so it reads as a distinct paragraph from the source above it.
const BILINGUAL_CSS: &str = "<style type=\"text/css\">.etc-trans{color:#222;margin:.15em 0 .9em;padding-left:.6em;border-left:2px solid #5B5BD6;}</style>";

/// CSS for annotation blocks. Vermilion (朱印紅) accent — the 眉批 metaphor —
/// deliberately distinct from the translation's indigo so the two voices never
/// blur. Injected only when the document actually carries a note.
const NOTE_CSS: &str = "<style type=\"text/css\">.etc-note{color:#5c2d24;background:rgba(178,58,42,.055);margin:.15em 0 .9em;padding:.35em .6em;border-left:2px solid #b23a2a;font-size:.92em;}</style>";

/// Map a user-facing language label to a BCP-47 code for the `lang` attribute.
/// Unknown labels pass through (covers users who already pass a code like "en").
pub(crate) fn lang_attr(label: &str) -> String {
    match label.trim() {
        "繁體中文" | "繁体中文" => "zh-Hant",
        "简体中文" | "簡體中文" => "zh-Hans",
        "中文" => "zh",
        "English" | "英文" => "en",
        "日本語" | "日文" => "ja",
        "한국어" | "韓文" | "韩文" => "ko",
        other => other,
    }
    .to_string()
}

fn inject_css(xhtml: &str, css: &str) -> String {
    // Insert once before </head>; if there's no head, leave it (the translation
    // still renders, just without the accent styling).
    let lower = xhtml.to_ascii_lowercase();
    if let Some(pos) = lower.find("</head>") {
        let mut s = String::with_capacity(xhtml.len() + css.len());
        s.push_str(&xhtml[..pos]);
        s.push_str(css);
        s.push_str(&xhtml[pos..]);
        s
    } else {
        xhtml.to_string()
    }
}

/// Apply translations + annotations and write a new EPUB to `out`. `lang`
/// labels translated nodes in bilingual mode; `note_lang` labels annotation
/// blocks (None = notes follow the reader profile's language, no lang attr).
pub fn write(
    doc: &EpubDoc,
    book: &Book,
    out: &Path,
    mode: OutputMode,
    lang: &str,
    note_lang: Option<&str>,
) -> Result<()> {
    let lang = lang_attr(lang);
    let note_lang = note_lang.map(lang_attr);
    // Build href -> serialized translated XHTML.
    let mut rendered: HashMap<String, String> = HashMap::new();
    // Index chapters by href once (the loop below would be O(chapters × docs)).
    let chapter_index: HashMap<&str, &Chapter> =
        book.chapters.iter().map(|c| (c.href.as_str(), c)).collect();
    for (href, dom) in &doc.docs {
        let mut dom = dom.clone();
        let mut touched = false;
        let mut noted = false;
        if let Some(chapter) = chapter_index.get(href.as_str()) {
            let mut targets: HashMap<usize, String> = HashMap::new();
            let mut notes: HashMap<usize, crate::document::Note> = HashMap::new();
            for seg in &chapter.segments {
                if let Some(t) = &seg.target {
                    let escaped = escape_min(t);
                    targets.insert(
                        seg.block_index,
                        crate::format::typography::normalize(
                            &placeholder::restore(&escaped, &seg.placeholders),
                            &lang,
                        ),
                    );
                }
                // Notes are model-authored plain text: escaped like targets, but
                // never placeholder-restored (a note owns no inline markup). The
                // placement (AN-014) rides along for the DOM to honour.
                if let Some(n) = seg.note.as_ref().filter(|n| !n.is_skip()) {
                    notes.insert(
                        seg.block_index,
                        crate::document::Note::new(n.pos, escape_min(&n.text)),
                    );
                }
            }
            touched = !targets.is_empty();
            noted = !notes.is_empty();
            dom.apply_segments(&targets, &notes, mode, &lang, note_lang.as_deref());
        }
        let mut xhtml = dom.serialize();
        if touched && mode == OutputMode::Bilingual {
            xhtml = inject_css(&xhtml, BILINGUAL_CSS);
        }
        if noted {
            xhtml = inject_css(&xhtml, NOTE_CSS);
        }
        rendered.insert(href.clone(), xhtml);
    }

    let buf = Vec::new();
    let mut zip = zip::ZipWriter::new(Cursor::new(buf));
    let stored: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // mimetype must be first and stored.
    if let Some((_, b)) = doc.entries.iter().find(|(n, _)| n == "mimetype") {
        zip.start_file("mimetype", stored)?;
        zip.write_all(b)?;
    }

    for (name, bytes) in &doc.entries {
        if name == "mimetype" {
            continue;
        }
        zip.start_file(name.clone(), deflated)?;
        if let Some(text) = rendered.get(name) {
            zip.write_all(text.as_bytes())?;
        } else {
            zip.write_all(bytes)?;
        }
    }

    let cursor = zip.finish()?;
    super::atomic_write(out, &cursor.into_inner())?;
    Ok(())
}

/// Minimal XML escaping for translated text (placeholders contain only [0-9/C],
/// so escaping `& < >` never disturbs them).
fn escape_min(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Security regression (audit H3): translated text is escaped before it goes
    // into the output XHTML, so a prompt-injected / malicious-book `<script>` or
    // event-handler can never survive as live markup that a downstream reader
    // could execute. Only the original inline tags re-enter via placeholder
    // restore (trusted, byte-faithful) — never new markup from the model.
    #[test]
    fn translated_text_cannot_inject_live_markup() {
        let out = escape_min("<script>alert(1)</script> & <img src=x onerror=evil>");
        assert!(!out.contains("<script"), "script tag must be neutralized");
        assert!(
            !out.contains("<img"),
            "img/event-handler tag must be neutralized"
        );
        assert!(out.contains("&lt;script&gt;"));
        assert!(out.contains("&amp;"));
    }

    // ---- malicious-input guards (audit 2026-07: zip bomb / zip-slip) ----

    /// Build a zip on disk from (name, bytes) entries; returns its path.
    fn write_zip(name: &str, entries: &[(&str, &[u8])]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (n, b) in entries {
            zip.start_file(n.to_string(), opts).unwrap();
            zip.write_all(b).unwrap();
        }
        zip.finish().unwrap();
        path
    }

    /// Minimal-but-valid EPUB entry set plus any extra entries.
    fn epub_entries<'a>(extra: &[(&'a str, &'a [u8])]) -> Vec<(&'a str, &'a [u8])> {
        let mut v: Vec<(&str, &[u8])> = vec![
            ("mimetype", b"application/epub+zip"),
            (
                "META-INF/container.xml",
                br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
            ),
            (
                "OEBPS/content.opf",
                br#"<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#,
            ),
            (
                "OEBPS/ch1.xhtml",
                br#"<html><body><p>Hello.</p></body></html>"#,
            ),
        ];
        v.extend_from_slice(extra);
        v
    }

    /// A prefixed OPF must read exactly like an unprefixed one. `<opf:item>`
    /// and `<item>` are the same element — the prefix is arbitrary — and real
    /// toolchains emit both. This locks the call site (`extract`), not just
    /// `parse_opf`: the bug shipped as "0 chapters, $0.00, completed".
    #[test]
    fn prefixed_opf_reads_the_same_as_an_unprefixed_one() {
        let plain = write_zip("tx-opf-plain.epub", &epub_entries(&[]));
        let (plain_book, _) = extract(&plain).unwrap();

        let prefixed_opf: &[u8] = br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest><opf:item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></opf:manifest><opf:spine><opf:itemref idref="c1"/></opf:spine></opf:package>"#;
        let mut entries = epub_entries(&[]);
        for e in entries.iter_mut() {
            if e.0 == "OEBPS/content.opf" {
                *e = ("OEBPS/content.opf", prefixed_opf);
            }
        }
        let prefixed = write_zip("tx-opf-prefixed.epub", &entries);
        let (prefixed_book, _) = extract(&prefixed).unwrap();

        assert_eq!(prefixed_book.chapters.len(), plain_book.chapters.len());
        assert_eq!(
            prefixed_book.chapters[0].segments.len(),
            plain_book.chapters[0].segments.len()
        );
    }

    /// An unreadable book must fail, not report a finished $0.00 run: a caller
    /// (and the MCP server) cannot tell "0 chapters, completed" apart from
    /// "already translated".
    #[test]
    fn a_book_with_no_chapters_is_an_error_not_a_free_success() {
        let empty_opf: &[u8] =
            br#"<package xmlns="http://www.idpf.org/2007/opf"><manifest/><spine/></package>"#;
        let mut entries = epub_entries(&[]);
        for e in entries.iter_mut() {
            if e.0 == "OEBPS/content.opf" {
                *e = ("OEBPS/content.opf", empty_opf);
            }
        }
        let path = write_zip("tx-opf-empty.epub", &entries);
        match extract(&path) {
            Err(CoreError::MalformedEpub(_)) => {}
            Err(other) => panic!("expected MalformedEpub, got {other:?}"),
            Ok(_) => panic!("expected an error for a book with no chapters"),
        }
    }

    #[test]
    fn zip_bomb_per_entry_cap_rejected() {
        // 64 KiB of zeros deflates to ~100 bytes: a miniature bomb. With a
        // 1 KiB per-entry cap the reader must reject it instead of buffering.
        let big = vec![0u8; 64 * 1024];
        let path = write_zip(
            "et_sec_bomb_entry.epub",
            &epub_entries(&[("OEBPS/bomb.bin", &big[..])]),
        );
        let limits = Limits {
            max_file_bytes: Limits::default().max_file_bytes,
            max_entries: 65_536,
            max_entry_bytes: 1024,
            max_total_bytes: 1024 * 1024,
        };
        let err = match extract_with_limits(&path, &limits) {
            Ok(_) => panic!("bomb must be rejected"),
            Err(e) => e,
        };
        let _ = std::fs::remove_file(&path);
        assert!(
            err.to_string()
                .contains("per-entry decompressed size limit"),
            "expected the per-entry cap to fire, got: {err}"
        );
    }

    #[test]
    fn zip_bomb_total_cap_rejected() {
        // Many mid-size entries, each under the per-entry cap, must still trip
        // the archive-wide budget.
        let chunk = vec![0u8; 800];
        let extra: Vec<(&str, &[u8])> = vec![
            ("OEBPS/a.bin", &chunk[..]),
            ("OEBPS/b.bin", &chunk[..]),
            ("OEBPS/c.bin", &chunk[..]),
            ("OEBPS/d.bin", &chunk[..]),
        ];
        let path = write_zip("et_sec_bomb_total.epub", &epub_entries(&extra));
        let limits = Limits {
            max_file_bytes: Limits::default().max_file_bytes,
            max_entries: 65_536,
            max_entry_bytes: 1024,
            max_total_bytes: 2048,
        };
        let err = match extract_with_limits(&path, &limits) {
            Ok(_) => panic!("bomb must be rejected"),
            Err(e) => e,
        };
        let _ = std::fs::remove_file(&path);
        assert!(
            err.to_string().contains("total decompressed size limit"),
            "expected the total cap to fire, got: {err}"
        );
    }

    #[test]
    fn zip_entry_count_cap_rejected() {
        let path = write_zip("et_sec_many_entries.epub", &epub_entries(&[]));
        let limits = Limits {
            max_file_bytes: Limits::default().max_file_bytes,
            max_entries: 2,
            max_entry_bytes: 1024 * 1024,
            max_total_bytes: 1024 * 1024,
        };
        let err = match extract_with_limits(&path, &limits) {
            Ok(_) => panic!("entry-count bomb must be rejected"),
            Err(e) => e,
        };
        let _ = std::fs::remove_file(&path);
        assert!(err.to_string().contains("entries (limit 2)"));
    }

    #[test]
    fn zip_slip_entry_never_touches_filesystem() {
        // An entry named `../…` must not be materialized anywhere: extraction
        // is in-memory only. The escape target must not exist afterwards.
        let escape_target = std::env::temp_dir()
            .parent()
            .unwrap_or(std::path::Path::new("/"))
            .join("et_sec_zip_slip_escape.txt");
        let _ = std::fs::remove_file(&escape_target);
        let path = write_zip(
            "et_sec_zip_slip.epub",
            &epub_entries(&[("../et_sec_zip_slip_escape.txt", b"pwned")]),
        );
        let res = extract(&path);
        let _ = std::fs::remove_file(&path);
        assert!(
            !escape_target.exists(),
            "zip-slip entry must never be written to disk"
        );
        // The book itself still parses, but the hostile entry is dropped rather
        // than carried — see `hostile_entry_names_cannot_ride_into_the_output`.
        let (book, doc) = res.unwrap();
        assert_eq!(book.chapters.len(), 1);
        assert!(
            !doc.entries.iter().any(|(n, _)| n.contains("..")),
            "a traversal entry must not survive into the IR"
        );
    }

    /// Reading was always safe (nothing is extracted), but the writer rebuilds
    /// the archive from the entry names it was given. Before this guard, a
    /// hostile name survived translation verbatim, so the *output* book was a
    /// zip-slip payload signed, in effect, by a tool the reader trusts.
    #[test]
    fn hostile_entry_names_cannot_ride_into_the_output() {
        let hostile: &[(&str, &[u8])] = &[
            ("../../../../tmp/et_slip_relative.txt", b"pwned"),
            ("/tmp/et_slip_absolute.txt", b"pwned"),
            ("a\\..\\..\\et_slip_backslash.txt", b"pwned"),
            ("C:/et_slip_drive.txt", b"pwned"),
            ("./et_slip_dot.txt", b"pwned"),
        ];
        let path = write_zip("et_sec_slip_roundtrip.epub", &epub_entries(hostile));
        let (book, doc) = extract(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(book.chapters.len(), 1, "the book must still translate");

        let out = std::env::temp_dir().join("et_sec_slip_roundtrip.out.epub");
        let _ = std::fs::remove_file(&out);
        write(&doc, &book, &out, OutputMode::Replace, "繁體中文", None).unwrap();

        let f = std::fs::File::open(&out).unwrap();
        let archive = zip::ZipArchive::new(f).unwrap();
        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        let _ = std::fs::remove_file(&out);

        for (bad, _) in hostile {
            assert!(
                !names.iter().any(|n| n == bad),
                "hostile entry {bad:?} survived into the output archive: {names:?}"
            );
        }
        // And nothing that merely *looks* traversal-ish slipped through either.
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with('/') || n.starts_with('\\') || n.contains("..")),
            "output archive still carries an unsafe name: {names:?}"
        );
        // The legitimate structure is untouched.
        assert!(names.iter().any(|n| n == "mimetype"));
    }

    #[test]
    fn containable_entry_accepts_real_epub_paths() {
        for ok in [
            "mimetype",
            "META-INF/container.xml",
            "EPUB/text/ch001.xhtml",
            "OEBPS/images/cover.jpg",
            "a..b/c.xhtml", // `..` inside a component is not a traversal
            "...hidden.xhtml",
        ] {
            assert!(is_containable_entry(ok), "should accept {ok}");
        }
        for bad in [
            "",
            "/abs.txt",
            "\\abs.txt",
            "C:/drive.txt",
            "../up.txt",
            "a/../../up.txt",
            "a\\..\\up.txt",
            "./here.txt",
            "a/./b.txt",
        ] {
            assert!(!is_containable_entry(bad), "should reject {bad}");
        }
    }

    /// Every other cap describes decompressed bytes and therefore cannot fire
    /// until the file is already resident. A large file with an `.epub` name is
    /// the cheapest possible attack, and it does not even need to be a zip.
    #[test]
    fn an_oversized_file_is_refused_before_it_is_read() {
        let path = std::env::temp_dir().join("et_sec_huge.epub");
        std::fs::write(&path, vec![0u8; 4096]).unwrap();
        let limits = Limits {
            max_file_bytes: 1024, // below the 4 KiB we just wrote
            ..Limits::default()
        };
        let err = extract_with_limits(&path, &limits)
            .err()
            .expect("an oversized file must be refused");
        let _ = std::fs::remove_file(&path);
        assert!(
            err.to_string().contains("refusing to read it into memory"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn normal_epub_passes_default_limits() {
        let path = write_zip("et_sec_normal.epub", &epub_entries(&[]));
        let res = extract(&path);
        let _ = std::fs::remove_file(&path);
        let (book, _) = res.unwrap();
        assert_eq!(book.chapters.len(), 1);
        assert_eq!(book.chapters[0].segments.len(), 1);
    }
}
