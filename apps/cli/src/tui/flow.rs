//! Screen flow: pick a book, choose what to do with it, agree to the cost.
//!
//! The flow only ever *collects intent*. It hands fully-formed args back to the
//! caller and gets out of the way, so the interactive path and the flag-driven
//! path converge on exactly one implementation of the run itself.
//!
//! The shape treats translation and margin notes as
//! two services you switch on independently, and turning translation off while
//! leaving notes on is how you say "annotate only". Every per-book answer is
//! remembered in the shared settings file.

use super::i18n::{tr, tr1};
use super::widgets::{self, Field, FormExit, Item, Kind, List, Menu};
use super::{resume, store, term};
use anyhow::Result;
use et_core::settings::{BookAnnotationSettings, Settings};
use std::path::{Path, PathBuf};

/// What the user decided to do.
pub enum Intent {
    Run(Choices),
    /// Price it, then come back to the menu.
    Estimate(Choices),
    /// A screen finished and the menu should be redrawn.
    Continue,
    Quit,
}

/// Everything a run needs, collected interactively.
pub struct Choices {
    pub input: PathBuf,
    pub translate: bool,
    pub annotate: bool,
    pub to: String,
    pub level: String,
    pub mode: String,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub profile: Option<String>,
    pub note_level: Option<String>,
    pub note_anchors: Option<String>,
    pub note_voice: Option<String>,
    pub note_presets: Option<String>,
    pub note_lang: Option<String>,
    pub density: String,
    /// An explicit output path, or None to let the run derive it next to the
    /// input.
    pub output: Option<std::path::PathBuf>,
}

/// Languages offered up front. Not a closed set — the engine takes any label —
/// but a menu needs a starting point, and these cover the corpus the product
/// was designed against.
const LANGS: &[&str] = &["繁體中文", "简体中文", "English", "日本語", "한국어"];

/// The help angles the annotation engine knows about. Ids are the engine's;
/// the second column is what the reader is actually choosing.
const PRESET_IDS: &[&str] = &[
    "terms",
    "history",
    "author",
    "culture",
    "characters",
    "concepts",
    "world",
    "methods",
    "research",
];
fn preset_label(id: &str) -> &'static str {
    match id {
        "terms" => tr("svc.terms"),
        "history" => tr("svc.history"),
        "author" => tr("svc.author"),
        "culture" => tr("svc.culture"),
        "characters" => tr("svc.characters"),
        "concepts" => tr("svc.concepts"),
        "world" => tr("svc.world"),
        "methods" => tr("svc.methods"),
        _ => tr("svc.research"),
    }
}

/// Where the model comes from. The classic three-way choice
/// : a subscription you already pay for, your own API key, or a local
/// model. `mock` is the fourth because a free offline dry run is genuinely
/// useful before committing to a real one.
const SOURCE_IDS: &[&str] = &["subscription", "api key", "ollama"];
fn source_label(id: &str) -> &'static str {
    match id {
        "subscription" => tr("src.subscription"),
        "api key" => tr("src.apikey"),
        _ => tr("src.ollama"),
    }
}
fn source_help(id: &str) -> String {
    match id {
        "subscription" => format!(
            "{}: {}",
            tr("src.subscription.sub"),
            tr("src.subscription.cost")
        ),
        "api key" => format!("{}: {}", tr("src.apikey.sub"), tr("src.apikey.cost")),
        _ => format!("{}: {}", tr("src.ollama.sub"), tr("src.ollama.cost")),
    }
}
fn source_id_of_label(label: &str) -> &'static str {
    SOURCE_IDS
        .iter()
        .find(|id| source_label(id) == label)
        .copied()
        .unwrap_or("api key")
}

const DENSITY_IDS: &[&str] = &["sparse", "medium", "rich"];
fn density_label(engine: &str) -> &'static str {
    match engine {
        "sparse" => tr("cfg.density.sparse"),
        "rich" => tr("cfg.density.rich"),
        _ => tr("cfg.density.medium"),
    }
}
fn density_engine(label: &str) -> &'static str {
    DENSITY_IDS
        .iter()
        .find(|id| density_label(id) == label)
        .copied()
        .unwrap_or("medium")
}

/// The engine's provider id for a chosen source, plus its default base URL.
fn source_to_provider(source: &str, settings: &Settings) -> (String, Option<String>) {
    // A stored base URL belongs to the source it was saved under; switching
    // sources must not drag it along (a sidecar URL is wrong for a paid API
    // key, and the other way round).
    let inherited = (provider_to_source(settings) == source)
        .then(|| settings.api.base_url.clone())
        .flatten();
    match source {
        "subscription" => (
            "openai".into(),
            Some(inherited.unwrap_or_else(|| "http://127.0.0.1:8765/v1".into())),
        ),
        "api key" => ("openai".into(), inherited),
        "ollama" => ("ollama".into(), None),
        _ => ("openai".into(), None),
    }
}

/// Which source label the stored settings represent. Settings saved by this
/// version carry it explicitly; older files fall back to inference.
fn provider_to_source(settings: &Settings) -> String {
    if let Some(s) = settings.api.source.as_deref() {
        if SOURCE_IDS.contains(&s) {
            return s.into();
        }
    }
    match settings.api.provider.as_str() {
        "ollama" => "ollama".into(),
        // The factory default provider is mock (an engine-level dry-run tool);
        // the interactive session does not offer it — a fresh install lands on
        // the subscription source.
        "mock" => "subscription".into(),
        _ => {
            let sidecar = settings
                .api
                .base_url
                .as_deref()
                .is_some_and(|u| u.contains(":8765"));
            if sidecar {
                "subscription".into()
            } else {
                "api key".into()
            }
        }
    }
}

/// Curated per source. A free-text model box would be more flexible and much
/// worse: the common failure here is a typo'd model id that only surfaces as a
/// 404 after the run starts.
/// The picker's options for a source, with `current` guaranteed to be among
/// them. A scan that misses the user's own model must never silently swap it:
/// the model is part of the cache signature, so a swap throws away every
/// finished chapter and can point the run at a model that is not installed.
fn models_for(source: &str, current: &str) -> Vec<String> {
    let mut list = models_catalog(source);
    if !current.is_empty() && !list.iter().any(|m| m == current) {
        list.insert(0, current.to_string());
    }
    list
}

fn models_catalog(source: &str) -> Vec<String> {
    match source {
        "api key" => vec![
            "gpt-5.4-mini".into(),
            "gpt-5.4".into(),
            "gpt-5.4-nano".into(),
        ],
        // A curated subset of what the sidecar's /v1/models actually serves —
        // the registry is authoritative; keep this list inside it.
        // Current generation first: whatever sits at the top is what a new
        // install lands on, and the previous generation sat there long after it
        // stopped being the best model the same subscription could reach. The
        // older ids stay selectable — pinning one is a legitimate choice (a job
        // cache is keyed by model, so switching restarts a book).
        "subscription" => vec![
            "claude-sonnet-5".into(),
            "claude-opus-5".into(),
            "claude-haiku-4-5".into(),
            "claude-sonnet-4-6".into(),
            "gpt-5.5".into(),
            "gpt-5.4".into(),
        ],
        _ => ollama_models(),
    }
}

/// The models actually installed in the local Ollama — a curated guess is
/// wrong the moment the user's tags differ (`qwen2.5:7b` vs `qwen2.5`), and a
/// picker that offers models you don't have is worse than none. Falls back to
/// common names when `ollama` isn't on PATH or doesn't answer quickly.
fn ollama_models() -> Vec<String> {
    let fallback = || vec!["llama3.3".into(), "qwen2.5".into(), "gemma3".into()];
    let Ok(out) = std::process::Command::new("ollama")
        .arg("list")
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return fallback();
    };
    if !out.status.success() {
        return fallback();
    }
    let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1) // header row
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        fallback()
    } else {
        names
    }
}

fn is_codex_subscription(source: &str, model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    source == "subscription"
        && (model.starts_with("gpt")
            || model.starts_with("codex")
            || model
                .strip_prefix('o')
                .and_then(|rest| rest.chars().next())
                .is_some_and(|c| c.is_ascii_digit()))
}

/// Books we can find without being told. Looks in the working directory and
/// one level below it, newest first — the book you just downloaded is almost
/// always the one you want.
fn discover(root: &Path) -> Vec<PathBuf> {
    fn is_book(p: &Path) -> bool {
        matches!(
            p.extension()
                .and_then(|e| e.to_str())
                .map(str::to_lowercase)
                .as_deref(),
            Some("epub") | Some("txt")
        )
    }
    let mut found = Vec::new();
    let mut dirs = vec![(root.to_path_buf(), 0u8)];
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                // Skip the obvious build/dependency sinks; a `node_modules`
                // walk would take longer than the translation.
                if depth < 1 && !matches!(name.as_ref(), "node_modules" | "target" | "venv") {
                    dirs.push((p, depth + 1));
                }
            } else if is_book(&p) {
                found.push(p);
            }
        }
    }
    found.sort_by_key(|p| {
        std::cmp::Reverse(
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        )
    });
    found
}

fn human_size(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{} KB", (bytes / 1024).max(1))
    }
}

fn index_of(items: &[&str], want: &str) -> usize {
    items.iter().position(|i| *i == want).unwrap_or(0)
}

/// Where the output lands by default: next to the book, named for what was
/// done to it. Shown to the user rather than left implicit, because "where did
/// my file go" is the first question after a long run.
fn default_output_path(input: &Path, to: &str, translate: bool) -> std::path::PathBuf {
    // The run's own derivation, not a lookalike: the placeholder on screen, the
    // file that appears, and the job cache resume reads are all the same path
    // or they are a bug.
    if translate {
        crate::default_output(input, to)
    } else {
        crate::default_annotate_output(input)
    }
}

/// The book-list detail column: how big the book is, and — when a job cache
/// beside it says so — how far a previous run got.
fn list_detail(input: &Path, to: &str, translate: bool) -> String {
    let size = std::fs::metadata(input)
        .map(|m| human_size(m.len()))
        .unwrap_or_default();
    match resume_hint(input, to, translate) {
        Some(hint) => format!("{size}  ·  {hint}"),
        None => size,
    }
}

/// "8/12 chapters done" for a book with unfinished work, `None` otherwise.
///
/// Probed against the job path the *defaults* derive — which is what the
/// config screen opens with. Change the target language or the "Save to" on
/// that screen and the run moves to a different job; the confirm gate is where
/// that gets said, because it is the screen that knows the final answers.
fn resume_hint(input: &Path, to: &str, translate: bool) -> Option<String> {
    let job = resume::job_path(input, to, translate, None);
    let r = resume::Resume::probe(&job)?;
    if !r.unfinished() {
        return None;
    }
    Some(match r.total {
        Some(total) => tr("list.resume")
            .replacen("{}", &r.done_count().to_string(), 1)
            .replacen("{}", &total.to_string(), 1),
        None => tr1("list.resume.nototal", &r.done_count().to_string()),
    })
}

/// Expand a leading `~` — people paste `~/Books` and shells are not here to
/// expand it for them.
fn expand_tilde(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(raw)
}

/// What a typed path turned out to be.
enum PathEntry {
    Book(PathBuf),
    Folder(PathBuf),
}

/// Ask for a path to a book or a folder of books. Loops on invalid input;
/// Esc returns None.
fn ask_for_path() -> Result<Option<PathEntry>> {
    loop {
        let mut fields = vec![
            Field::header(""),
            Field::text(
                tr("path.field"),
                "",
                tr("path.placeholder"),
                tr("path.help"),
            ),
        ];
        let mut no_refresh = |_: &mut [Field]| {};
        match widgets::form_with(
            tr("path.title"),
            tr("path.tip"),
            &mut fields,
            Some(tr("path.open")),
            &mut no_refresh,
        )? {
            FormExit::Back => return Ok(None),
            FormExit::Action(_) => {}
            FormExit::Submit => {
                let raw = fields[1].text_value();
                let raw = raw.trim();
                if raw.is_empty() {
                    continue;
                }
                let path = expand_tilde(raw);
                if path.is_dir() {
                    return Ok(Some(PathEntry::Folder(path)));
                }
                let is_book = matches!(
                    path.extension()
                        .and_then(|e| e.to_str())
                        .map(str::to_lowercase)
                        .as_deref(),
                    Some("epub") | Some("txt")
                );
                if path.is_file() && is_book {
                    return Ok(Some(PathEntry::Book(path)));
                }
                widgets::notice(
                    tr("path.bad.title"),
                    &[
                        format!("  {}", term::sanitize_plain(&path.to_string_lossy())),
                        String::new(),
                        if path.exists() {
                            tr("path.bad.onlyepub").to_string()
                        } else {
                            tr("path.bad.missing").to_string()
                        },
                    ],
                )?;
            }
        }
    }
}

/// Run the interactive flow once. Returns what the user wants to do next.
pub fn run(root: &Path) -> Result<Intent> {
    let mut settings = store::load();

    let menu_items = [
        (tr("menu.translate"), tr("menu.translate.sub")),
        (tr("menu.annotate"), tr("menu.annotate.sub")),
        (tr("menu.estimate"), tr("menu.estimate.sub")),
        (tr("menu.settings"), tr("menu.settings.sub")),
    ];
    let choice = Menu {
        title: tr("menu.title"),
        items: &menu_items,
    }
    .run()?;

    let Some(choice) = choice else {
        return Ok(Intent::Quit);
    };

    if choice == 3 {
        settings_screen(&mut settings)?;
        return Ok(Intent::Continue);
    }

    let mut books = discover(root);
    if let Some(extra) = settings
        .general
        .books_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        for b in discover(&expand_tilde(extra)) {
            if !books.contains(&b) {
                books.push(b);
            }
        }
    }
    if books.is_empty() {
        widgets::notice(
            tr("nobooks.title"),
            &[
                tr("nobooks.l1").to_string(),
                tr1("nobooks.l2", &term::sanitize_plain(&root.to_string_lossy())),
                String::new(),
                tr("nobooks.l3").to_string(),
                tr("nobooks.l4").to_string(),
            ],
        )?;
        match ask_for_path()? {
            None => return Ok(Intent::Continue),
            Some(PathEntry::Book(p)) => books = vec![p],
            Some(PathEntry::Folder(d)) => {
                books = discover(&d);
                if books.is_empty() {
                    widgets::notice(
                        tr("folder.empty.title"),
                        &[tr1(
                            "folder.empty.l1",
                            &term::sanitize_plain(&d.to_string_lossy()),
                        )],
                    )?;
                    return Ok(Intent::Continue);
                }
            }
        }
    }

    // Esc undoes exactly one step: config returns to the book list, the book
    // list returns to the menu, the menu quits. Collapsing any of those into
    // "back to the start" makes a mis-keyed Esc cost the user their setup.
    // "Annotate" from the menu is the same config screen with the services
    // pre-flipped, not a second implementation — and it writes to a different
    // default output, so it reads a different job cache.
    let annotate_only = choice == 1;
    let default_to = settings.general.default_target_lang.clone();
    loop {
        let mut items: Vec<Item> = books
            .iter()
            .map(|p| Item {
                label: p
                    .file_name()
                    .map(|n| term::sanitize_plain(&n.to_string_lossy()))
                    .unwrap_or_default(),
                detail: list_detail(p, &default_to, !annotate_only),
            })
            .collect();
        items.push(Item {
            label: tr("list.elsewhere").to_string(),
            detail: tr("list.elsewhere.sub").to_string(),
        });
        let picked = List {
            title: tr("list.title"),
            items: items.clone(),
            empty: tr("list.empty"),
            pseudo_tail: 1,
        }
        .run()?;
        let Some(picked) = picked else {
            return Ok(Intent::Continue);
        };
        let input: PathBuf = if picked == books.len() {
            match ask_for_path()? {
                None => continue,
                // A directly-opened book goes straight to its config screen
                // (and joins the list for when the user Escs back to it).
                Some(PathEntry::Book(p)) => {
                    if !books.contains(&p) {
                        books.insert(0, p.clone());
                    }
                    p
                }
                Some(PathEntry::Folder(d)) => {
                    let found = discover(&d);
                    if found.is_empty() {
                        widgets::notice(
                            tr("folder.empty.title"),
                            &[tr1(
                                "folder.empty.l1",
                                &term::sanitize_plain(&d.to_string_lossy()),
                            )],
                        )?;
                    } else {
                        books = found;
                    }
                    continue;
                }
            }
        } else {
            books[picked].clone()
        };

        match config_screen(&input, &mut settings, annotate_only)? {
            Some(c) if choice == 2 => return Ok(Intent::Estimate(c)),
            Some(c) => return Ok(Intent::Run(c)),
            // Back out of the config screen: choose a different book.
            None => continue,
        }
    }
}

// Row indices into the config form. Named because a bare `fields[7]` three
// screens from its construction is how a form quietly starts reading the wrong
// value after someone inserts a row.
const F_TRANSLATE: usize = 1;
const F_INTO: usize = 2;
const F_DEPTH: usize = 3;
const F_LAYOUT: usize = 4;
const F_ANNOTATE: usize = 6;
const F_PRESETS: usize = 7;
const F_PROFILE: usize = 8;
const F_ANCHORS: usize = 9;
const F_LEVEL: usize = 10;
const F_VOICE: usize = 11;
const F_DENSITY: usize = 12;
const F_NOTELANG: usize = 13;
const F_SOURCE: usize = 15;
const F_MODEL: usize = 16;
const F_OUTPUT: usize = 18; // after the Output header at 17

/// The per-book config screen: two
/// switchable services on one page, at least one of which must be on.
/// The config form's rows, in order. Extracted so the index constants above
/// can be tested against the real thing rather than a copy of it.
fn build_config_fields(input: &Path, settings: &mut Settings, annotate_only: bool) -> Vec<Field> {
    let book = store::book_settings(settings, input);
    let source = provider_to_source(settings);
    let models = models_for(&source, &settings.api.model);
    let model_refs: Vec<&str> = models.iter().map(|m| m.as_str()).collect();

    let source_labels: Vec<&str> = SOURCE_IDS.iter().map(|id| source_label(id)).collect();
    let density_labels: Vec<&str> = DENSITY_IDS.iter().map(|id| density_label(id)).collect();
    let note_langs: Vec<&str> = std::iter::once(tr("cfg.notelang.auto"))
        .chain(LANGS.iter().copied())
        .collect();

    let preset_options: Vec<(String, String)> = PRESET_IDS
        .iter()
        .map(|id| (id.to_string(), preset_label(id).to_string()))
        .collect();
    let preset_chosen: Vec<bool> = PRESET_IDS
        .iter()
        .map(|id| book.presets.iter().any(|p| p == id))
        .collect();

    vec![
        Field::header(tr("cfg.h.translation")),
        Field::toggle(
            tr("cfg.translate"),
            if annotate_only { false } else { book.translate },
            tr("cfg.translate.help"),
        ),
        Field::choice(
            tr("cfg.into"),
            LANGS,
            index_of(LANGS, &settings.general.default_target_lang),
            tr("cfg.into.help"),
        ),
        Field::choice(
            tr("cfg.depth"),
            &[tr("cfg.depth.standard"), tr("cfg.depth.expert")],
            usize::from(settings.general.default_mode == "expert"),
            tr("cfg.depth.help"),
        ),
        Field::choice(
            tr("cfg.layout"),
            &[tr("cfg.layout.only"), tr("cfg.layout.side")],
            usize::from(settings.general.output == "bilingual"),
            tr("cfg.layout.help"),
        ),
        Field::header(tr("cfg.h.notes")),
        Field::toggle(
            tr("cfg.notes"),
            if annotate_only { true } else { book.annotate },
            tr("cfg.notes.help"),
        ),
        Field::multi(
            tr("cfg.services"),
            preset_options,
            preset_chosen,
            tr("cfg.services.placeholder"),
            tr("cfg.services.help"),
        ),
        Field::paragraph(
            tr("cfg.why"),
            &book.reader_profile,
            tr("cfg.why.placeholder"),
            tr("cfg.why.help"),
        ),
        Field::text(
            tr("cfg.anchors"),
            &book.anchors.join(", "),
            tr("cfg.anchors.placeholder"),
            tr("cfg.anchors.help"),
        ),
        Field::choice(
            tr("cfg.level"),
            &[
                tr("cfg.level.beginner"),
                tr("cfg.level.general"),
                tr("cfg.level.insider"),
            ],
            match book.level.as_str() {
                "beginner" => 0,
                "insider" => 2,
                _ => 1,
            },
            tr("cfg.level.help"),
        ),
        Field::choice(
            tr("cfg.voice"),
            &[tr("cfg.voice.study"), tr("cfg.voice.companion")],
            usize::from(book.voice == "companion"),
            tr("cfg.voice.help"),
        ),
        Field::choice(
            tr("cfg.density"),
            &density_labels,
            index_of(&density_labels, density_label(&book.density)),
            tr("cfg.density.help"),
        ),
        Field::choice(tr("cfg.notelang"), &note_langs, 0, tr("cfg.notelang.help")),
        Field::header(tr("cfg.h.model")),
        Field::choice(
            tr("cfg.source"),
            &source_labels,
            SOURCE_IDS.iter().position(|i| *i == source).unwrap_or(0),
            &source_help(&source),
        ),
        Field::choice(
            tr("cfg.model"),
            &model_refs,
            index_of(&model_refs, &settings.api.model),
            "",
        ),
        Field::header(tr("cfg.h.output")),
        Field::text(
            tr("cfg.saveto"),
            "",
            &term::sanitize_plain(
                &default_output_path(input, &settings.general.default_target_lang, !annotate_only)
                    .to_string_lossy(),
            ),
            tr("cfg.saveto.help"),
        ),
    ]
}

fn config_screen(
    input: &Path,
    settings: &mut Settings,
    annotate_only: bool,
) -> Result<Option<Choices>> {
    let mut fields = build_config_fields(input, settings, annotate_only);

    let name = input
        .file_name()
        .map(|n| term::sanitize_plain(&n.to_string_lossy()))
        .unwrap_or_default();

    // Runs before every repaint, so a section dims the moment its switch flips
    // rather than on the next visit to the screen.
    let input_path = input.to_path_buf();
    let mut refresh = |fields: &mut [Field]| {
        let translate_on = fields[F_TRANSLATE].is_on();
        let notes_on = fields[F_ANNOTATE].is_on();
        for f in &mut fields[F_INTO..=F_LAYOUT] {
            f.disabled = !translate_on;
        }
        for f in &mut fields[F_PRESETS..=F_NOTELANG] {
            f.disabled = !notes_on;
        }
        // The model list has to track the source, or switching to Ollama leaves
        // an OpenAI model id selected.
        let picked = fields[F_MODEL].current().to_string();
        let valid = models_for(source_id_of_label(fields[F_SOURCE].current()), &picked);
        if let Kind::Choice { choices, value } = &mut fields[F_MODEL].kind {
            if *choices != valid {
                // Keep the selection by name. Re-indexing to 0 would silently
                // change the model the run bills and invalidate its cache.
                let keep = valid.iter().position(|m| *m == picked).unwrap_or(0);
                *choices = valid;
                *value = keep;
            }
        }
        // The placeholder always shows where the file will actually land, so a
        // change of target language or of translate-vs-annotate is visible
        // before the run rather than a surprise after it.
        let to = fields[F_INTO].current().to_string();
        let translating = fields[F_TRANSLATE].is_on();
        let default = term::sanitize_plain(
            &default_output_path(&input_path, &to, translating).to_string_lossy(),
        );
        if let Kind::Text { empty, .. } = &mut fields[F_OUTPUT].kind {
            *empty = default;
        }
    };

    loop {
        match widgets::form_with(
            tr("cfg.title"),
            &name,
            &mut fields,
            Some(tr("cfg.continue")),
            &mut refresh,
        )? {
            FormExit::Back => return Ok(None),
            FormExit::Action(_) => {}
            FormExit::Submit => {
                let translate = fields[F_TRANSLATE].is_on();
                let annotate = fields[F_ANNOTATE].is_on();
                if !translate && !annotate {
                    widgets::notice(
                        tr("cfg.nothing.title"),
                        &[
                            tr("cfg.nothing.l1").to_string(),
                            String::new(),
                            tr("cfg.nothing.l2").to_string(),
                        ],
                    )?;
                    continue;
                }
                if annotate
                    && fields[F_PRESETS].picked().is_empty()
                    && fields[F_PROFILE].text_value().trim().is_empty()
                {
                    widgets::notice(
                        tr("cfg.who.title"),
                        &[
                            tr("cfg.who.l1").to_string(),
                            tr("cfg.who.l2").to_string(),
                            tr("cfg.who.l3").to_string(),
                            String::new(),
                            tr("cfg.who.l4").to_string(),
                            tr("cfg.who.l5").to_string(),
                        ],
                    )?;
                    continue;
                }

                let source = source_id_of_label(fields[F_SOURCE].current()).to_string();
                let (provider, base_url) = source_to_provider(&source, settings);
                let model = fields[F_MODEL].current().to_string();
                if is_codex_subscription(&source, &model) {
                    widgets::notice(
                        tr("cfg.codex.title"),
                        &[
                            tr("cfg.codex.l1").to_string(),
                            tr("cfg.codex.l2").to_string(),
                            tr("cfg.codex.l3").to_string(),
                        ],
                    )?;
                }
                let profile = fields[F_PROFILE].text_value();
                // Cycler values are self-describing ("study (quiet)"); the
                // engine id is the first word.
                let level = if fields[F_LEVEL].current() == tr("cfg.level.beginner") {
                    "beginner".to_string()
                } else if fields[F_LEVEL].current() == tr("cfg.level.insider") {
                    "insider".to_string()
                } else {
                    "general".to_string()
                };
                let anchors: Vec<String> = fields[F_ANCHORS]
                    .text_value()
                    .split(',')
                    .map(|a| a.trim().to_string())
                    .filter(|a| !a.is_empty())
                    .collect();
                let voice = if fields[F_VOICE].current() == tr("cfg.voice.companion") {
                    "companion".to_string()
                } else {
                    "study".to_string()
                };
                let presets = fields[F_PRESETS].picked();
                let density = density_engine(fields[F_DENSITY].current()).to_string();
                let note_lang = fields[F_NOTELANG].current().to_string();
                let output = {
                    let typed = fields[F_OUTPUT].text_value();
                    (!typed.trim().is_empty()).then(|| std::path::PathBuf::from(typed.trim()))
                };

                // Remember this book's answers, the way the app does.
                store::remember_book(
                    settings,
                    input,
                    BookAnnotationSettings {
                        translate,
                        annotate,
                        reader_profile: profile.clone(),
                        level: level.clone(),
                        anchors: anchors.clone(),
                        voice: voice.clone(),
                        presets: presets.clone(),
                        density: density.clone(),
                        updated_at: 0,
                    },
                );
                settings.api.provider = provider.clone();
                settings.api.source = Some(source.clone());
                settings.api.model = model.clone();
                let _ = store::save(settings);

                return Ok(Some(Choices {
                    input: input.to_path_buf(),
                    translate,
                    annotate,
                    to: fields[F_INTO].current().to_string(),
                    level: if fields[F_DEPTH].current() == tr("cfg.depth.expert") {
                        "expert".into()
                    } else {
                        "sentence".into()
                    },
                    mode: if fields[F_LAYOUT].current() == tr("cfg.layout.side") {
                        "bilingual".into()
                    } else {
                        "replace".into()
                    },
                    provider,
                    model,
                    base_url,
                    profile: (!profile.trim().is_empty()).then_some(profile),
                    note_level: (level != "general").then_some(level),
                    note_anchors: (!anchors.is_empty()).then(|| anchors.join(",")),
                    note_voice: (voice != "study").then_some(voice),
                    note_presets: (!presets.is_empty()).then(|| presets.join(",")),
                    note_lang: (note_lang != tr("cfg.notelang.auto")).then_some(note_lang),
                    density,
                    output,
                }));
            }
        }
    }
}

// Row indices into the settings form.
const S_SOURCE: usize = 1;
const S_MODEL: usize = 2;
const S_BASEURL: usize = 3;
const S_KEY: usize = 4;
const S_TEST: usize = 5;
const S_FORGET: usize = 6;
const S_LANG: usize = 8;
const S_DEPTH: usize = 9;
const S_LAYOUT: usize = 10;
const S_BOOKSDIR: usize = 11;
const S_PROFILE: usize = 12;
const S_PRESETS: usize = 13;
const S_DENSITY: usize = 14;

/// The settings form's rows, in order. Extracted so the index constants above
/// are checked against the real form rather than a copy of it.
fn build_settings_fields(settings: &Settings, key_hint: Option<String>) -> Vec<Field> {
    let source_labels: Vec<&str> = SOURCE_IDS.iter().map(|id| source_label(id)).collect();
    let density_labels: Vec<&str> = DENSITY_IDS.iter().map(|id| density_label(id)).collect();
    let source = provider_to_source(settings);
    let models = models_for(&source, &settings.api.model);
    let model_refs: Vec<&str> = models.iter().map(|m| m.as_str()).collect();
    let hint = key_hint;
    let source_help_text = source_help(&source);
    let preset_options: Vec<(String, String)> = PRESET_IDS
        .iter()
        .map(|id| (id.to_string(), preset_label(id).to_string()))
        .collect();
    let preset_chosen: Vec<bool> = PRESET_IDS
        .iter()
        .map(|id| settings.annotations.presets.iter().any(|p| p == id))
        .collect();
    vec![
        Field::header(tr("set.h.source")),
        Field::choice(
            tr("cfg.source"),
            &source_labels,
            SOURCE_IDS.iter().position(|i| *i == source).unwrap_or(0),
            &source_help_text,
        ),
        Field::choice(
            tr("cfg.model"),
            &model_refs,
            index_of(&model_refs, &settings.api.model),
            "",
        ),
        Field::text(
            tr("set.baseurl"),
            settings.api.base_url.as_deref().unwrap_or(""),
            tr("set.baseurl.placeholder"),
            tr("set.baseurl.help"),
        ),
        {
            // Subscription mode stores the sidecar's access token in the same
            // slot; label it as what the user actually pastes.
            let (label, help) = if source == "subscription" {
                (tr("set.token"), tr("set.token.help"))
            } else {
                (tr("set.key"), tr("set.key.help"))
            };
            Field::secret(label, hint, help)
        },
        Field::action(tr("set.test"), tr("set.test.help")),
        Field::action(tr("set.forget"), tr("set.forget.help")),
        Field::header(tr("set.h.newbooks")),
        Field::choice(
            tr("set.into"),
            LANGS,
            index_of(LANGS, &settings.general.default_target_lang),
            "",
        ),
        Field::choice(
            tr("cfg.depth"),
            &[tr("cfg.depth.standard"), tr("cfg.depth.expert")],
            usize::from(settings.general.default_mode == "expert"),
            "",
        ),
        Field::choice(
            tr("cfg.layout"),
            &[tr("cfg.layout.only"), tr("cfg.layout.side")],
            usize::from(settings.general.output == "bilingual"),
            "",
        ),
        Field::text(
            tr("set.booksdir"),
            settings.general.books_dir.as_deref().unwrap_or(""),
            tr("set.booksdir.placeholder"),
            tr("set.booksdir.help"),
        ),
        Field::paragraph(
            tr("set.whyread"),
            &settings.annotations.reader_profile,
            tr("set.key.notset"),
            tr("set.whyread.help"),
        ),
        Field::multi(
            tr("cfg.services"),
            preset_options,
            preset_chosen,
            tr("set.services.placeholder"),
            tr("set.services.help"),
        ),
        Field::choice(
            tr("set.notedensity"),
            &density_labels,
            index_of(
                &density_labels,
                density_label(&settings.annotations.density),
            ),
            "",
        ),
    ]
}

/// Settings: where the model comes from, and what new books start with.
/// The settings panel.
fn settings_screen(settings: &mut Settings) -> Result<()> {
    // `None` = whatever the keychain says; `Some(_)` = we changed it this visit.
    let mut saved_key: Option<bool> = None;
    loop {
        let source = provider_to_source(settings);
        let keyed_provider = "openai";

        // Never inline: see store::key_hint_nonblocking. `saved_key` tracks
        // edits made on this screen, which the session cache cannot see.
        let hint = match saved_key {
            Some(true) => Some(tr("set.key.saved").to_string()),
            Some(false) => Some(tr("set.key.notset").to_string()),
            None => {
                store::key_hint_nonblocking(keyed_provider).map(|h| tr1("set.key.savedhint", &h))
            }
        };
        let mut fields = build_settings_fields(settings, hint);

        // Ollama needs no key. Subscription mode DOES need one: the sidecar
        // prints a local access token at startup and rejects requests without
        // it — disabling the row here would leave a fresh user with no way in.
        let needs_key = matches!(source.as_str(), "api key" | "subscription");
        fields[S_KEY].disabled = !needs_key;
        fields[S_FORGET].disabled = !needs_key;
        fields[S_BASEURL].disabled = matches!(source.as_str(), "mock" | "ollama");

        let exit = widgets::form(
            tr("set.title"),
            tr("set.subtitle"),
            &mut fields,
            Some(tr("set.save")),
        )?;

        // Read the form back before acting, so triggering an action does not
        // discard edits made on the same visit.
        let picked_source = source_id_of_label(fields[S_SOURCE].current()).to_string();
        let (provider, default_base) = source_to_provider(&picked_source, settings);
        settings.api.provider = provider;
        settings.api.source = Some(picked_source.clone());
        if !fields[S_MODEL].current().is_empty() {
            settings.api.model = fields[S_MODEL].current().to_string();
        }
        let typed_base = fields[S_BASEURL].text_value();
        settings.api.base_url = if typed_base.trim().is_empty() {
            default_base
        } else {
            Some(typed_base.trim().to_string())
        };
        settings.general.default_target_lang = fields[S_LANG].current().to_string();
        settings.general.default_mode = if fields[S_DEPTH].current() == tr("cfg.depth.expert") {
            "expert".into()
        } else {
            "sentence".into()
        };
        settings.general.output = if fields[S_LAYOUT].current() == tr("cfg.layout.side") {
            "bilingual".into()
        } else {
            "replace".into()
        };
        let typed_books_dir = fields[S_BOOKSDIR].text_value();
        settings.general.books_dir =
            (!typed_books_dir.trim().is_empty()).then(|| typed_books_dir.trim().to_string());
        settings.annotations.reader_profile = fields[S_PROFILE].text_value();
        settings.annotations.presets = fields[S_PRESETS].picked();
        settings.annotations.density = density_engine(fields[S_DENSITY].current()).to_string();

        match exit {
            FormExit::Back | FormExit::Submit => {
                if let Err(e) = store::save(settings) {
                    widgets::notice(
                        tr("set.save.err.title"),
                        &[
                            format!("  {e}"),
                            String::new(),
                            format!("  {}", tr("set.save.err.l1")),
                        ],
                    )?;
                }
                return Ok(());
            }
            FormExit::Action(i) => {
                match i {
                    S_KEY => {
                        // Same keychain slot either way; the words differ
                        // because the thing pasted differs (a provider API key
                        // vs the sidecar's printed access token).
                        let (title, prompt) = if picked_source == "subscription" {
                            (tr("set.token"), tr("set.token.dialog.prompt"))
                        } else {
                            (tr("set.key"), tr("set.key.dialog.prompt"))
                        };
                        if let Some(k) =
                            widgets::input(title, prompt, "", true, &[tr("set.key.dialog.note")])?
                        {
                            if !k.trim().is_empty() {
                                match et_core::secrets::set_key(keyed_provider, k.trim()) {
                                    Ok(()) => saved_key = Some(true),
                                    Err(e) => {
                                        widgets::notice(
                                            tr("set.key.err.title"),
                                            &[format!("  {e}")],
                                        )?;
                                    }
                                }
                            }
                        }
                    }
                    S_TEST => {
                        let lines = test_connection(settings);
                        widgets::notice(tr("set.test.title"), &lines)?;
                    }
                    S_FORGET => {
                        let _ = et_core::secrets::delete_key(keyed_provider);
                        saved_key = Some(false);
                        widgets::notice(
                            tr("set.forget.done.title"),
                            &[format!("  {}", tr("set.forget.done.l1"))],
                        )?;
                    }
                    _ => {}
                }
                let _ = store::save(settings);
            }
        }
    }
}

/// One real request against the configured endpoint.
///
/// Reports what actually came back rather than "OK": a connection test that
/// only proves a socket opened is the kind of check that passes while the thing
/// it tests is broken.
fn test_connection(settings: &Settings) -> Vec<String> {
    // The TUI runs inside the CLI's tokio runtime; building another runtime on
    // this thread panics ("Cannot start a runtime from within a runtime"), so
    // the probe gets its own OS thread with its own runtime.
    let settings = settings.clone();
    std::thread::spawn(move || test_connection_blocking(&settings))
        .join()
        .unwrap_or_else(|_| vec![format!("  {} internal error", super::theme::FAIL)])
}

/// Label column for the connection test. `{:<10}` pads by char count, so a
/// 12-char label like "Access token" got no padding at all and ran straight
/// into its value ("Access tokenpresent"); the translated labels are worse
/// still (`アクセストークン` is 8 chars but 16 columns). Pad by display width,
/// and always leave at least one space so the row can never close up.
fn label_col(label: &str) -> String {
    const TARGET: usize = 14;
    let w = super::term::width(label);
    format!("{label}{}", " ".repeat(TARGET.saturating_sub(w).max(1)))
}

fn test_connection_blocking(settings: &Settings) -> Vec<String> {
    use et_core::config::{Level, ProviderKind, TranslateConfig};
    use et_core::llm::{ChatMessage, CompletionRequest, Provider};

    let mut out = Vec::new();
    let provider_kind: ProviderKind = match settings.api.provider.parse() {
        Ok(p) => p,
        Err(e) => return vec![format!("  {e}")],
    };
    let mut cfg = TranslateConfig::new("English");
    cfg.provider = provider_kind;
    cfg.model = settings.api.model.clone();
    cfg.level = Level::Sentence;

    // Same order as the run itself resolves credentials: an env var on the
    // invocation beats the stored key, otherwise the test would pass against
    // one credential and the run would use another.
    let key = std::env::var("OPENAI_API_KEY")
        .ok()
        .or_else(|| et_core::secrets::get_key("openai").ok().flatten());

    out.push(format!(
        "  {}{}",
        label_col(tr("test.endpoint")),
        settings
            .api
            .base_url
            .as_deref()
            .unwrap_or(tr("test.default"))
    ));
    out.push(format!("  {}{}", label_col(tr("test.model")), cfg.model));
    out.push(format!(
        "  {}{}",
        label_col(if provider_to_source(settings) == "subscription" {
            tr("set.token")
        } else {
            tr("test.keyrow")
        }),
        if key.is_some() {
            tr("test.present")
        } else {
            tr("test.absent")
        }
    ));
    out.push(String::new());

    let provider = match Provider::from_config(&cfg, key, settings.api.base_url.clone()) {
        Ok(p) => p,
        Err(e) => {
            out.push(format!("  {} {e}", super::theme::FAIL));
            return out;
        }
    };
    let req = CompletionRequest {
        system: "Reply with the single word: ok".into(),
        messages: vec![ChatMessage::user("ping")],
        temperature: 0.0,
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            out.push(format!("  {} {e}", super::theme::FAIL));
            return out;
        }
    };
    match rt.block_on(provider.complete(&req)) {
        Ok(r) => {
            out.push(format!(
                "  {} {}",
                super::theme::OK,
                tr1("test.ok", &format!("{}/{}", r.tokens_in, r.tokens_out))
            ));
            let reply = r.text.trim();
            if !reply.is_empty() {
                out.push(format!(
                    "  {}",
                    tr1("test.reply", super::term::truncate(reply, 60).as_str())
                ));
            }
        }
        Err(e) => {
            out.push(format!("  {} {e}", super::theme::FAIL));
            out.push(String::new());
            out.push(format!("  {}", tr("test.sidecar.hint")));
            out.push("    cd apps/subscription-kit && npm start".into());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_labels_round_trip_through_the_engine_ids() {
        super::super::i18n::force(super::super::i18n::Lang::En);
        for engine in DENSITY_IDS {
            assert_eq!(density_engine(density_label(engine)), *engine);
        }
        // Anything unrecognised lands on the documented default rather than
        // panicking on a settings file written by a newer version.
        assert_eq!(density_engine("nonsense"), "medium");
        assert_eq!(density_label("nonsense"), density_label("medium"));
    }

    #[test]
    fn every_offered_source_maps_to_a_provider_the_engine_accepts() {
        use et_core::config::ProviderKind;
        let s = Settings::default();
        for id in SOURCE_IDS {
            let (provider, _) = source_to_provider(id, &s);
            assert!(
                provider.parse::<ProviderKind>().is_ok(),
                "source {id:?} produced an unknown provider {provider:?}"
            );
            assert!(
                !models_for(id, "").is_empty(),
                "source {id:?} offers no models"
            );
        }
    }

    #[test]
    fn a_saved_model_survives_a_scan_that_does_not_know_it() {
        // The local Ollama scan can miss (binary not on PATH, daemon slow) and
        // fall back to curated names. If the picker then dropped the user's own
        // model, opening the config screen would silently re-point the run: the
        // model is in the cache signature, so every finished chapter is thrown
        // away, and the substituted model may not even be installed.
        let list = models_for("ollama", "qwen2.5:3b");
        assert!(
            list.iter().any(|m| m == "qwen2.5:3b"),
            "the configured model has to stay on offer, got {list:?}"
        );
        assert_eq!(
            index_of(
                &list.iter().map(String::as_str).collect::<Vec<_>>(),
                "qwen2.5:3b"
            ),
            list.iter().position(|m| m == "qwen2.5:3b").unwrap(),
            "and it must resolve to its own row, never to row 0"
        );
    }

    #[test]
    fn an_unknown_model_is_not_duplicated_when_the_scan_does_know_it() {
        let known = models_catalog("api key")[0].clone();
        let list = models_for("api key", &known);
        assert_eq!(
            list.iter().filter(|m| **m == known).count(),
            1,
            "a model already in the catalog must not be inserted twice"
        );
    }

    #[test]
    fn codex_reminder_is_contextual_to_subscription_models() {
        assert!(is_codex_subscription("subscription", "gpt-5.4"));
        assert!(is_codex_subscription("subscription", "codex-mini"));
        assert!(is_codex_subscription("subscription", "o3"));
        assert!(!is_codex_subscription("subscription", "claude-sonnet-4-6"));
        assert!(!is_codex_subscription("api key", "gpt-5.4"));
        assert!(!is_codex_subscription("ollama", "qwen2.5"));
    }

    /// The source shown must survive a round trip through what gets persisted,
    /// or reopening Settings silently moves the user somewhere they did not
    /// choose.
    #[test]
    fn a_chosen_source_is_what_comes_back() {
        for id in SOURCE_IDS {
            let mut s = Settings::default();
            let (provider, base) = source_to_provider(id, &s);
            s.api.provider = provider;
            s.api.base_url = base;
            assert_eq!(
                &provider_to_source(&s),
                id,
                "source {id:?} did not survive the round trip"
            );
        }
    }

    #[test]
    fn preset_ids_are_the_ones_the_engine_declares() {
        // The CLI must not offer help angles the annotation engine will
        // silently ignore.
        let declared: Vec<&str> = PRESET_IDS.to_vec();
        for id in &declared {
            assert!(
                et_core::annotate::prompt::PRESETS
                    .iter()
                    .any(|(pid, _)| pid == id),
                "{id} is offered by the CLI but unknown to the engine"
            );
        }
        assert_eq!(
            declared.len(),
            et_core::annotate::prompt::PRESETS.len(),
            "the CLI and the engine disagree on how many help angles exist"
        );
    }

    /// Every field index constant must point at the row it is named for. The
    /// form is built as one long vector and read back by index; an inserted row
    /// would otherwise silently shift meanings without failing to compile.
    #[test]
    fn output_defaults_next_to_the_input() {
        let input = Path::new("/books/great-expectations.epub");
        let t = default_output_path(input, "繁體中文", true);
        assert_eq!(t.parent(), input.parent(), "must sit beside the book");
        assert_eq!(
            t.file_name().unwrap().to_string_lossy(),
            "great-expectations.繁體中文.epub"
        );
        // Annotate-only keeps the source text, so it is named for that.
        let a = default_output_path(input, "繁體中文", false);
        assert_eq!(
            a.file_name().unwrap().to_string_lossy(),
            "great-expectations.annotated.epub"
        );
        assert_eq!(a.parent(), input.parent());

        let hostile = default_output_path(input, "/../../victim", true);
        assert_eq!(hostile.parent(), input.parent());
        assert_eq!(
            hostile.file_name().unwrap().to_string_lossy(),
            "great-expectations.victim.epub"
        );
    }

    #[test]
    fn config_form_indices_match_their_labels() {
        super::super::i18n::force(super::super::i18n::Lang::En);
        let expect = [
            (F_TRANSLATE, "Translate"),
            (F_INTO, "Into"),
            (F_DEPTH, "Depth"),
            (F_LAYOUT, "Layout"),
            (F_ANNOTATE, "Add notes"),
            (F_PRESETS, "Help me with"),
            (F_PROFILE, "Why this book"),
            (F_ANCHORS, "You already know"),
            (F_LEVEL, "Explain for"),
            (F_VOICE, "Voice"),
            (F_DENSITY, "Density"),
            (F_NOTELANG, "Notes language"),
            (F_SOURCE, "Source"),
            (F_MODEL, "Model"),
            (F_OUTPUT, "Save to"),
        ];
        let mut s = Settings::default();
        let fields = build_config_fields(Path::new("/tmp/x.epub"), &mut s, false);
        for (i, label) in expect {
            assert_eq!(fields[i].label, label, "row {i} is not {label:?}");
        }
    }

    #[test]
    fn settings_form_indices_match_their_labels() {
        super::super::i18n::force(super::super::i18n::Lang::En);
        let expect = [
            (S_SOURCE, "Source"),
            (S_MODEL, "Model"),
            (S_BASEURL, "Base URL"),
            // A fresh install lands on the subscription source, where the
            // secret row is the sidecar's access token, not a provider key.
            (S_KEY, "Access token"),
            (S_TEST, "Test connection"),
            (S_FORGET, "Forget saved key"),
            (S_LANG, "Translate into"),
            (S_DEPTH, "Depth"),
            (S_LAYOUT, "Layout"),
            (S_BOOKSDIR, "Books folder"),
            (S_PROFILE, "Why I read"),
            (S_PRESETS, "Help me with"),
            (S_DENSITY, "Note density"),
        ];
        let s = Settings::default();
        let fields = build_settings_fields(&s, None);
        for (i, label) in expect {
            assert_eq!(fields[i].label, label, "row {i} is not {label:?}");
        }
    }

    /// The sidecar port is configurable (LLM_SUB_KIT_PORT); classifying the
    /// source by sniffing ":8765" out of the URL silently rewrote any other
    /// port back to the default. The explicit `source` field must win.
    #[test]
    fn explicit_source_survives_a_custom_sidecar_port() {
        let mut s = Settings::default();
        s.api.provider = "openai".into();
        s.api.source = Some("subscription".into());
        s.api.base_url = Some("http://127.0.0.1:8770/v1".into());
        assert_eq!(provider_to_source(&s), "subscription");
        let (provider, base) = source_to_provider("subscription", &s);
        assert_eq!(provider, "openai");
        assert_eq!(base.as_deref(), Some("http://127.0.0.1:8770/v1"));
    }

    /// Switching sources must not drag the other source's base URL along: a
    /// sidecar URL is wrong for a paid API key and the other way round.
    #[test]
    fn switching_sources_drops_the_stored_base_url() {
        let mut s = Settings::default();
        s.api.provider = "openai".into();
        s.api.source = Some("subscription".into());
        s.api.base_url = Some("http://127.0.0.1:8765/v1".into());
        let (_, base) = source_to_provider("api key", &s);
        assert_eq!(base, None);

        s.api.source = Some("api key".into());
        s.api.base_url = Some("https://api.example.com/v1".into());
        let (_, base) = source_to_provider("subscription", &s);
        assert_eq!(base.as_deref(), Some("http://127.0.0.1:8765/v1"));
        // ...while staying on the same source keeps what the user typed.
        let (_, base) = source_to_provider("api key", &s);
        assert_eq!(base.as_deref(), Some("https://api.example.com/v1"));
    }

    /// The Settings screen's connection test runs inside the CLI's tokio
    /// runtime (main is #[tokio::main]). It used to build a second runtime on
    /// the same thread, which panics — crashing the whole session on the one
    /// button that exists to build confidence. Probe an unreachable endpoint
    /// from inside a runtime and require lines back, not a panic.
    #[tokio::test(flavor = "multi_thread")]
    async fn connection_test_does_not_panic_inside_the_runtime() {
        let mut s = Settings::default();
        s.api.provider = "openai".into();
        s.api.source = Some("api key".into());
        s.api.base_url = Some("http://127.0.0.1:9/v1".into());
        s.api.model = "gpt-5.4-mini".into();
        let lines = test_connection(&s);
        assert!(!lines.is_empty());
        assert!(lines.iter().any(|l| l.contains("127.0.0.1:9")));
    }

    /// The book list is the first place "this book is half done" can show up,
    /// and the only cost it may pay for that is a `stat` per book.
    #[test]
    fn the_book_list_shows_unfinished_work_and_only_that() {
        let dir = std::env::temp_dir().join(format!("translatus-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let book = dir.join("b.epub");
        std::fs::write(&book, b"not really an epub").unwrap();

        // No job beside it: the row says nothing extra, and probing it must not
        // create one.
        assert!(resume_hint(&book, "繁體中文", true).is_none());
        let job = resume::job_path(&book, "繁體中文", true, None);
        assert!(!job.exists(), "listing a book must not create a job cache");

        {
            let store = et_core::job::JobStore::open(&job).unwrap();
            store.set_meta("total_chapters", "12").unwrap();
            for i in 0..8 {
                store
                    .set_chapter_status(i, &format!("c{i}.xhtml"), "done")
                    .unwrap();
            }
        }
        // Digits, not words: the row is translated five ways.
        let hint = resume_hint(&book, "繁體中文", true).expect("8 of 12 chapters are done");
        assert!(hint.contains('8') && hint.contains("12"), "{hint}");
        assert!(list_detail(&book, "繁體中文", true).contains(&hint));

        // A different target language is a different job, so no resume there.
        assert!(resume_hint(&book, "日本語", true).is_none());
        // …and so is annotate-only, which writes to `.annotated.`.
        assert!(resume_hint(&book, "繁體中文", false).is_none());
    }

    /// Settings written before `source` existed still classify sensibly.
    #[test]
    fn legacy_settings_without_source_fall_back_to_inference() {
        let mut s = Settings::default();
        s.api.provider = "openai".into();
        s.api.source = None;
        s.api.base_url = Some("http://127.0.0.1:8765/v1".into());
        assert_eq!(provider_to_source(&s), "subscription");
        s.api.base_url = Some("https://api.example.com/v1".into());
        assert_eq!(provider_to_source(&s), "api key");
    }
}
