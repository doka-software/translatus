//! End-to-end annotation tests on the offline mock provider: annotate-only and
//! translate+annotate produce valid EPUBs (etc-note present, source
//! byte-faithful), placement (AN-014) puts before-notes ahead of their
//! paragraph, the program-side chapter cap (AN-013) trims a greedy model, the
//! TXT path renders 〔註〕 lines, a second run costs zero LLM tokens, and the
//! unification review demonstrably drops duplicates.
//!
//! Mock determinism this file leans on: the selection pass picks every 3rd
//! pending unit (alternating before/after, decreasing priority) — or EVERY
//! unit when a text carries `SELECT_ALL_TOKEN` — and the writer notes every
//! selected unit with `〈註〉背景補充：<text head>`.

use et_core::config::{AnnotationConfig, Density, OutputMode, TranslateConfig};
use et_core::{format, job, translate};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("et-anno-e2e-{}-{}", std::process::id(), name))
}

fn cleanup(paths: &[&Path]) {
    for p in paths {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", p.display(), suffix));
        }
    }
}

/// Build a minimal, valid EPUB: mimetype + container + OPF + one XHTML per
/// chapter, each paragraph a `<p>`.
fn make_epub(path: &Path, chapters: &[Vec<&str>]) {
    let f = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(f);
    let stored: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated: zip::write::FileOptions<()> = zip::write::FileOptions::default();

    zip.start_file("mimetype", stored).unwrap();
    zip.write_all(b"application/epub+zip").unwrap();

    zip.start_file("META-INF/container.xml", deflated).unwrap();
    zip.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();

    let mut manifest = String::new();
    let mut spine = String::new();
    for i in 0..chapters.len() {
        manifest.push_str(&format!(
            r#"<item id="c{i}" href="c{i}.xhtml" media-type="application/xhtml+xml"/>"#
        ));
        spine.push_str(&format!(r#"<itemref idref="c{i}"/>"#));
    }
    let opf = format!(
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Test Book</dc:title></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>"#
    );
    zip.start_file("OEBPS/content.opf", deflated).unwrap();
    zip.write_all(opf.as_bytes()).unwrap();

    for (i, paras) in chapters.iter().enumerate() {
        let body: String = paras.iter().map(|p| format!("<p>{p}</p>")).collect();
        let doc = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>c{i}</title></head><body>{body}</body></html>"#
        );
        zip.start_file(format!("OEBPS/c{i}.xhtml"), deflated)
            .unwrap();
        zip.write_all(doc.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

/// Concatenated XHTML text of every chapter in an EPUB.
fn epub_text(path: &Path) -> String {
    let f = std::fs::File::open(path).unwrap();
    let mut zip = zip::ZipArchive::new(f).unwrap();
    let mut out = String::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        if entry.name().ends_with(".xhtml") {
            let mut s = String::new();
            entry.read_to_string(&mut s).unwrap();
            out.push_str(&s);
        }
    }
    out
}

fn anno_cfg() -> TranslateConfig {
    let mut cfg = TranslateConfig::new("繁體中文");
    cfg.annotations = Some(AnnotationConfig {
        reader_profile: "軟體工程師，第一次讀十九世紀小說，想理解當時的航海與捕鯨業背景。".into(),
        level: et_core::config::ExplainLevel::General,
        anchors: Vec::new(),
        voice: et_core::config::NoteVoice::Study,
        lang: Some("繁體中文".into()),
        density: Density::Medium,
        style: None,
        presets: Vec::new(),
    });
    cfg
}

const CH1: [&str; 5] = [
    "Call me Ishmael.",
    "Some years ago I went to sea.",
    "It is a way I have of driving off the spleen.",
    "There is nothing surprising in this.",
    "Almost all men cherish the same feelings towards the ocean.",
];
const CH2: [&str; 4] = [
    "The Pequod sailed at dawn.",
    "Captain Ahab stayed below deck.",
    "The crew whispered about his leg.",
    "Nantucket faded behind them.",
];

// Annotate-only: valid EPUB with etc-note blocks, byte-faithful source, no
// translation artifacts; the second run costs zero tokens (cache + review flag).
// Chapter caps here (medium, 5- and 4-segment chapters) are 1 each, so the two
// surviving notes are the chapters' first paragraphs — placed BEFORE them (the
// mock's first pick per chapter is a before-note).
#[tokio::test]
async fn annotate_only_epub_end_to_end() {
    let input = tmp("only.epub");
    let output = tmp("only.out.epub");
    let jobp = tmp("only.etjob");
    cleanup(&[&input, &output, &jobp]);
    make_epub(&input, &[CH1.to_vec(), CH2.to_vec()]);

    let cfg = anno_cfg();
    let provider = et_core::llm::Provider::from_config(&cfg, None, None).unwrap();

    // First run: notes get written.
    let (mut book, doc) = format::extract(&input).unwrap();
    let store = job::JobStore::open(&jobp).unwrap();
    let summary = translate::annotate_only(&provider, &cfg, &mut book, &store, |_| {})
        .await
        .unwrap();
    assert!(summary.notes_written > 0, "mock must produce notes");
    assert!(summary.tokens_in > 0, "first run does call the (mock) LLM");
    format::write(
        &doc,
        &book,
        &output,
        OutputMode::Replace,
        "原文",
        Some("繁體中文"),
    )
    .unwrap();

    let text = epub_text(&output);
    assert!(text.contains("etc-note"), "notes must be injected");
    assert!(text.contains("〔註〕"), "notes must self-identify");
    assert!(text.contains(".etc-note{"), "NOTE_CSS must be injected");
    assert!(
        !text.contains("etc-trans"),
        "annotate-only must not translate"
    );
    assert!(!text.contains("〈譯〉"), "annotate-only must not translate");
    for p in CH1.iter().chain(CH2.iter()) {
        assert!(
            text.contains(&format!("<p>{p}</p>")),
            "source paragraph must stay byte-faithful: {p}"
        );
    }
    // AN-013: the program-side cap (1 per chapter at this density/size) holds.
    assert_eq!(summary.notes_written, 2, "one note per chapter cap");
    assert_eq!(text.matches("etc-note\"").count(), 2);
    // AN-014: the surviving pick is a before-note — it precedes its paragraph.
    let note = text.find("〔註〕").unwrap();
    let para = text.find("<p>Call me Ishmael.</p>").unwrap();
    assert!(note < para, "before-note renders ahead of its paragraph");

    // Second run on the same job: every decision cached + review flagged done
    // → zero LLM tokens, same notes.
    let (mut book2, doc2) = format::extract(&input).unwrap();
    let summary2 = translate::annotate_only(&provider, &cfg, &mut book2, &store, |_| {})
        .await
        .unwrap();
    assert_eq!(summary2.tokens_in, 0, "resume must not call the LLM");
    assert_eq!(summary2.tokens_out, 0, "resume must not call the LLM");
    assert_eq!(summary2.notes_written, summary.notes_written);
    assert_eq!(
        summary2.notes_restored_from_cache,
        book2.total_segments(),
        "every per-segment decision (note or skip) restores from cache"
    );
    let output2 = tmp("only.out2.epub");
    cleanup(&[&output2]);
    format::write(
        &doc2,
        &book2,
        &output2,
        OutputMode::Replace,
        "原文",
        Some("繁體中文"),
    )
    .unwrap();
    assert_eq!(
        epub_text(&output2),
        text,
        "resumed output must be identical"
    );

    // Cache-only re-render (no provider in scope at all) restores the notes.
    let (mut book3, doc3) = format::extract(&input).unwrap();
    let anno_sig = store.get_meta("anno_sig").unwrap().expect("recorded");
    let rs = translate::render_from_cache(&mut book3, &store, None, Some(&anno_sig)).unwrap();
    // Prefill covers EVERY segment decision (incl. skips); the honest visible
    // count is notes_in_output — the two etc-note blocks, not 9 segments.
    assert_eq!(rs.note_segments_prefilled, book3.total_segments());
    assert_eq!(rs.notes_in_output, summary.notes_written);
    let output3 = tmp("only.out3.epub");
    cleanup(&[&output3]);
    format::write(
        &doc3,
        &book3,
        &output3,
        OutputMode::Replace,
        "原文",
        Some("繁體中文"),
    )
    .unwrap();
    assert_eq!(
        epub_text(&output3),
        text,
        "cache-only output must be identical"
    );

    cleanup(&[&input, &output, &output2, &output3, &jobp]);
}

// Translate + annotate in one run (bilingual): both voices present, ordered
// 原文 → 譯文 → 眉批, both CSS blocks injected, second run free.
#[tokio::test]
async fn translate_plus_annotate_bilingual_end_to_end() {
    let input = tmp("both.epub");
    let output = tmp("both.out.epub");
    let jobp = tmp("both.etjob");
    cleanup(&[&input, &output, &jobp]);
    make_epub(&input, &[CH1.to_vec(), CH2.to_vec()]);

    let mut cfg = anno_cfg();
    cfg.output_mode = OutputMode::Bilingual;
    let sig = cfg.cache_signature();
    let provider = et_core::llm::Provider::from_config(&cfg, None, None).unwrap();

    let (mut book, doc) = format::extract(&input).unwrap();
    let store = job::JobStore::open(&jobp).unwrap();
    let mut streamed_notes = 0usize;
    let summary = translate::run(&provider, &cfg, &mut book, &store, &sig, |p| {
        streamed_notes += p.notes.len();
    })
    .await
    .unwrap();
    assert!(summary.units_translated > 0);
    assert!(summary.notes_written > 0);
    assert_eq!(
        streamed_notes, summary.notes_written,
        "every written note streams exactly once for the read-along"
    );
    format::write(
        &doc,
        &book,
        &output,
        OutputMode::Bilingual,
        "繁體中文",
        Some("繁體中文"),
    )
    .unwrap();

    let text = epub_text(&output);
    assert!(text.contains("etc-trans") && text.contains("etc-note"));
    assert!(text.contains(".etc-trans{") && text.contains(".etc-note{"));
    // The first annotated paragraph carries a BEFORE note (mock's first pick):
    // order must be 眉批(before) → 原文 → 譯文 — the note never splits the
    // source/translation pair.
    let src = text.find("<p>Call me Ishmael.</p>").unwrap();
    let trans = text.find("〈譯〉Call me Ishmael.").unwrap();
    let note = text.find(r#"<div class="etc-note""#).unwrap();
    assert!(
        note < src && src < trans,
        "order must be 眉批(before)→原文→譯文"
    );
    for p in CH1.iter().chain(CH2.iter()) {
        assert!(text.contains(&format!("<p>{p}</p>")), "byte-faithful: {p}");
    }

    // Second run: translation cache + note cache + review flag → zero tokens.
    let (mut book2, _doc2) = format::extract(&input).unwrap();
    let summary2 = translate::run(&provider, &cfg, &mut book2, &store, &sig, |_| {})
        .await
        .unwrap();
    assert_eq!(summary2.tokens_in + summary2.tokens_out, 0);
    assert_eq!(summary2.notes_written, summary.notes_written);

    cleanup(&[&input, &output, &jobp]);
}

// TXT path: notes appear as 〔註〕 lines after their paragraph; source preserved.
#[tokio::test]
async fn annotate_txt_end_to_end() {
    let input = tmp("plain.txt");
    let output = tmp("plain.out.txt");
    let jobp = tmp("plain.etjob");
    cleanup(&[&input, &output, &jobp]);
    let paras = [
        "First paragraph about whaling history.",
        "Second paragraph on ship life.",
        "Third paragraph, quiet seas.",
        "Fourth paragraph, a storm gathers.",
    ];
    std::fs::write(&input, paras.join("\n\n")).unwrap();

    let cfg = anno_cfg();
    let provider = et_core::llm::Provider::from_config(&cfg, None, None).unwrap();
    let (mut book, doc) = format::extract(&input).unwrap();
    let store = job::JobStore::open(&jobp).unwrap();
    let summary = translate::annotate_only(&provider, &cfg, &mut book, &store, |_| {})
        .await
        .unwrap();
    assert!(summary.notes_written > 0);
    format::write(&doc, &book, &output, OutputMode::Replace, "原文", None).unwrap();

    let text = std::fs::read_to_string(&output).unwrap();
    assert!(text.contains("〔註〕 "), "TXT notes must be marked lines");
    for p in paras {
        assert!(text.contains(p), "source paragraph preserved: {p}");
    }
    // The surviving pick is a before-note: its marked line sits directly ABOVE
    // its paragraph (AN-014 on the TXT path).
    let first = text.find(paras[0]).unwrap();
    let note = text.find("〔註〕").unwrap();
    assert!(note < first, "before-note line precedes its paragraph");

    cleanup(&[&input, &output, &jobp]);
}

// The unification review: mock notes are derived from the paragraph text, so
// identical paragraphs yield identical notes — the review must drop the
// duplicates (observable via notes_dropped) and keep exactly one.
#[tokio::test]
async fn review_drops_duplicate_notes() {
    let input = tmp("dupes.epub");
    let output = tmp("dupes.out.epub");
    let jobp = tmp("dupes.etjob");
    cleanup(&[&input, &output, &jobp]);
    // Positions 0 and 3 (both picked by the mock's every-3rd rule) carry the
    // SAME text → duplicate notes before review. Rich density on 6 segments
    // gives a chapter cap of 2, so BOTH picks survive the cap and reach the
    // dedupe stage.
    let dup = "The White Whale breached again.";
    make_epub(
        &input,
        &[vec![
            dup,
            "Filler one, not annotated.",
            "Filler two, not annotated.",
            dup,
            "Filler three, not annotated.",
            "Filler four, not annotated.",
        ]],
    );

    let mut cfg = anno_cfg();
    cfg.annotations.as_mut().unwrap().density = Density::Rich;
    let provider = et_core::llm::Provider::from_config(&cfg, None, None).unwrap();
    let (mut book, doc) = format::extract(&input).unwrap();
    let store = job::JobStore::open(&jobp).unwrap();
    let summary = translate::annotate_only(&provider, &cfg, &mut book, &store, |_| {})
        .await
        .unwrap();
    assert!(summary.notes_dropped > 0, "duplicate note must be dropped");
    assert_eq!(summary.notes_written, 1, "exactly one survives the review");
    format::write(&doc, &book, &output, OutputMode::Replace, "原文", None).unwrap();
    let text = epub_text(&output);
    assert_eq!(
        text.matches("etc-note\"").count(),
        1,
        "output carries exactly one note block"
    );

    // The dropped decision is cached too: a re-run stays deduped at zero cost.
    let (mut book2, _) = format::extract(&input).unwrap();
    let summary2 = translate::annotate_only(&provider, &cfg, &mut book2, &store, |_| {})
        .await
        .unwrap();
    assert_eq!(summary2.tokens_in + summary2.tokens_out, 0);
    assert_eq!(summary2.notes_written, 1);
    assert_eq!(summary2.notes_dropped, 0, "nothing left to drop on resume");

    cleanup(&[&input, &output, &jobp]);
}

// AN-013 is code-enforced: with SELECT_ALL_TOKEN the mock "model" greedily
// selects all 30 paragraphs, yet exactly cap(medium, 30) = 2 notes come out —
// and they are the two highest-priority picks (the earliest paragraphs, since
// the mock's priorities decrease). Sparsity survives a maximally greedy model.
#[tokio::test]
async fn chapter_cap_trims_greedy_model_by_priority() {
    use et_core::llm::mock::SELECT_ALL_TOKEN;
    let input = tmp("cap.epub");
    let output = tmp("cap.out.epub");
    let jobp = tmp("cap.etjob");
    cleanup(&[&input, &output, &jobp]);
    let paras: Vec<String> = (0..30)
        .map(|i| format!("Paragraph {i:02} {SELECT_ALL_TOKEN} unique whaling fact number {i}."))
        .collect();
    make_epub(&input, &[paras.iter().map(String::as_str).collect()]);

    let cfg = anno_cfg(); // medium density
    let provider = et_core::llm::Provider::from_config(&cfg, None, None).unwrap();
    let (mut book, doc) = format::extract(&input).unwrap();
    let store = job::JobStore::open(&jobp).unwrap();
    let summary = translate::annotate_only(&provider, &cfg, &mut book, &store, |_| {})
        .await
        .unwrap();
    assert_eq!(
        summary.notes_written, 2,
        "30 greedy selections must be trimmed to the chapter cap of 2"
    );
    format::write(&doc, &book, &output, OutputMode::Replace, "原文", None).unwrap();
    let text = epub_text(&output);
    assert_eq!(text.matches("etc-note\"").count(), 2);
    // Priority order decides who survives: the two earliest paragraphs.
    assert!(text.contains("背景補充：Paragraph 00"));
    assert!(text.contains("背景補充：Paragraph 01"));
    assert!(!text.contains("背景補充：Paragraph 02"));

    cleanup(&[&input, &output, &jobp]);
}

// Full chain (select → write → review → export) with both placements: a rich
// 8-paragraph chapter caps at 2 — the mock picks paragraphs 0 (before) and
// 3 (after); the before-note must precede its paragraph, the after-note must
// follow its, and the remaining paragraphs carry no note at all.
#[tokio::test]
async fn before_and_after_placements_end_to_end() {
    let input = tmp("pos.epub");
    let output = tmp("pos.out.epub");
    let jobp = tmp("pos.etjob");
    cleanup(&[&input, &output, &jobp]);
    let paras: Vec<String> = (0..8)
        .map(|i| format!("Chapter fact {i} stands alone."))
        .collect();
    make_epub(&input, &[paras.iter().map(String::as_str).collect()]);

    let mut cfg = anno_cfg();
    cfg.annotations.as_mut().unwrap().density = Density::Rich; // cap = 2
    let provider = et_core::llm::Provider::from_config(&cfg, None, None).unwrap();
    let (mut book, doc) = format::extract(&input).unwrap();
    let store = job::JobStore::open(&jobp).unwrap();
    let summary = translate::annotate_only(&provider, &cfg, &mut book, &store, |_| {})
        .await
        .unwrap();
    assert_eq!(summary.notes_written, 2);
    format::write(&doc, &book, &output, OutputMode::Replace, "原文", None).unwrap();

    let text = epub_text(&output);
    assert_eq!(text.matches("etc-note\"").count(), 2, "6 段落無註");
    // before-note: ahead of paragraph 0.
    let note0 = text.find("背景補充：Chapter fact 0").unwrap();
    let para0 = text.find("<p>Chapter fact 0 stands alone.</p>").unwrap();
    assert!(note0 < para0, "before-note precedes its paragraph");
    // after-note: behind paragraph 3.
    let note3 = text.find("背景補充：Chapter fact 3").unwrap();
    let para3 = text.find("<p>Chapter fact 3 stands alone.</p>").unwrap();
    assert!(note3 > para3, "after-note follows its paragraph");
    // the note siblings never clone the source tag's ids and self-identify
    assert!(text.contains(r#"<div class="etc-note""#));

    // Placement survives the cache: a zero-token re-run reproduces the bytes.
    let (mut book2, doc2) = format::extract(&input).unwrap();
    let summary2 = translate::annotate_only(&provider, &cfg, &mut book2, &store, |_| {})
        .await
        .unwrap();
    assert_eq!(summary2.tokens_in + summary2.tokens_out, 0);
    let output2 = tmp("pos.out2.epub");
    cleanup(&[&output2]);
    format::write(&doc2, &book2, &output2, OutputMode::Replace, "原文", None).unwrap();
    assert_eq!(epub_text(&output2), text, "placement survives resume");

    cleanup(&[&input, &output, &output2, &jobp]);
}
