//! translatus — agent-native CLI. Structured JSON I/O, idempotent, resumable.
//! A thin wrapper over the `et-core` engine.

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use et_core::config::{
    AnnotationConfig, Density, Level, OutputMode, ProviderKind, TranslateConfig,
};
use et_core::{format, job, translate};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Capability-parity gate: CAPABILITIES.toml ↔ real CLI surface ↔ TranslateConfig.
#[cfg(test)]
mod parity_tests;

/// `translatus mcp` — stdio MCP server for agents (see module docs).
mod mcp;

/// Interactive front door: bare `translatus` on a terminal (see module docs).
mod tui;

/// Which face a run wears. The engine path is identical either way; this only
/// decides whether progress is appended as log lines or painted as a board.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ui {
    Cli,
    Tui,
}

/// Shown at the bottom of `translatus --help`: how to connect a model, in the
/// classic three-way split: subscription, API key, or local.
const HELP_PROVIDERS: &str = "\
Providers:
  --provider mock       offline dry-run of the full pipeline; free (default)
  --provider openai     any OpenAI-compatible endpoint; OS keychain or $OPENAI_API_KEY
  --provider ollama     local models, no key needed (http://localhost:11434/v1)

  Subscription (no API key) — start the local sidecar, then point at it:
    cd apps/subscription-kit && npm install && npm start
    export OPENAI_API_KEY='<local access token printed by npm start>'
    translatus translate book.epub --to English --provider openai \\
      --base-url http://127.0.0.1:8765/v1 --model <id from GET /v1/models>

Getting started:
  translatus                                      # interactive: pick a book and go
  translatus estimate book.epub --to English      # price a run before starting
  translatus translate book.epub --to English --provider ollama --model llama3.3
  Interrupted? Re-run the same command — cached segments are never re-billed.

Full guide: docs/GUIDE.md in the repo · FAQ: docs/FAQ.md";

#[derive(Parser)]
#[command(
    name = "translatus",
    version,
    about = "Translatus — from any book, to your book (BYO LLM)",
    after_help = HELP_PROVIDERS
)]
struct Cli {
    /// Emit machine-readable JSON (default for agents).
    #[arg(long, global = true)]
    json: bool,
    /// Omitted on a terminal, we open the interactive session instead.
    #[command(subcommand)]
    cmd: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Translate a book (idempotent; re-run to resume).
    Translate(TranslateArgs),
    /// Annotate a book for a specific reader WITHOUT translating it
    /// (output keeps the source text + margin notes; idempotent; re-run to resume).
    Annotate(AnnotateArgs),
    /// Estimate tokens/cost without translating.
    Estimate(EstimateArgs),
    /// Run as a stdio MCP server so agents (Claude Code, etc.) can call
    /// estimate_book / translate_book / annotate_book as tools.
    /// Run `translatus mcp install` once to register it with the agents you
    /// have installed; a bare `translatus mcp` is what they then launch.
    Mcp(McpArgs),
}

#[derive(Parser)]
struct McpArgs {
    /// Omitted = run the server on stdio (what an agent client invokes).
    #[command(subcommand)]
    cmd: Option<McpCommand>,
}

#[derive(Subcommand)]
enum McpCommand {
    /// Register this MCP server with the agent clients found on this machine.
    ///
    /// Deliberately a separate step, never part of `brew install`: writing into
    /// another program's configuration is something you ask for, not something
    /// a package manager does behind your back. Registration goes through each
    /// client's own CLI (`claude mcp add`, `codex mcp add`) rather than editing
    /// their config files, so their formats stay theirs to change.
    Install,
    /// Undo `install` for the clients found on this machine.
    Uninstall,
}

#[derive(Parser)]
struct TranslateArgs {
    /// Input file (.epub or .txt).
    input: PathBuf,
    /// Target language label, e.g. "繁體中文".
    #[arg(long)]
    to: String,
    #[arg(long, default_value = "sentence")]
    level: String,
    #[arg(long, default_value = "mock")]
    provider: String,
    #[arg(long, default_value = "mock")]
    model: String,
    /// Override provider base URL (OpenAI-compatible endpoints / local servers).
    #[arg(long)]
    base_url: Option<String>,
    /// Custom prompt text (injected into the style section).
    #[arg(long)]
    prompt: Option<String>,
    /// Output path. Defaults to <input>.<lang>.<ext>.
    #[arg(long)]
    output: Option<PathBuf>,
    /// replace | bilingual.
    #[arg(long, default_value = "replace")]
    mode: String,
    /// Max batches translated concurrently. Raise for direct API providers to
    /// finish faster; keep at 1 for local/subscription-style backends.
    #[arg(long, default_value_t = 1)]
    concurrency: usize,
    /// Job/cache DB path (resume key). Defaults to <output>.etjob.
    #[arg(long)]
    job: Option<PathBuf>,
    /// Re-render output from the cache only — no LLM calls, zero cost. Use to
    /// change layout/bilingual after a book is already translated.
    #[arg(long)]
    cache_only: bool,
    /// Also write reader-personalised margin notes. Requires --profile.
    #[arg(long)]
    annotate: bool,
    /// Reader background + motivation (free text, or @file to read a file).
    #[arg(long)]
    profile: Option<String>,
    /// Language the notes are written in. Defaults to the translation target
    /// language (translate) or the reader profile's own language (annotate).
    #[arg(long)]
    note_lang: Option<String>,
    /// Note style paragraph (tone / depth / length target; free text, or @file).
    /// Injected inside the locked hard rules; defaults to the engine style.
    #[arg(long)]
    note_style: Option<String>,
    /// The service menu, comma-separated ids: terms, history, author,
    /// culture, characters, concepts, world, methods, research. What the
    /// notes should do for you; with at least one picked, --profile becomes
    /// optional. Unknown ids are ignored with a warning.
    #[arg(long)]
    note_presets: Option<String>,
    /// Reader-profile document (讀者側寫契約): a JSON file path, or inline JSON
    /// starting with `{`. Fields: purpose, anchors, presets, voice, lang,
    /// density, style — all optional; explicit flags override. See
    /// docs/READER-PROFILE.md for the schema and the standard prompt that lets
    /// the reader's own AI fill it.
    #[arg(long)]
    note_profile: Option<String>,
    /// Cognitive anchors (認知錨), comma-separated short labels of what the
    /// reader already knows (e.g. "軟體工程師,讀過《國富論》"). Notes bridge
    /// new concepts FROM these; never quoted in the notes.
    #[arg(long)]
    note_anchors: Option<String>,
    /// Explanation level: beginner (入門白話: everyday language + examples,
    /// no jargon assumed) | general (default) | insider (內行: only what an
    /// insider could not easily look up).
    #[arg(long)]
    note_level: Option<String>,
    /// Note voice register: study (書齋, default) | companion (陪讀).
    #[arg(long)]
    note_voice: Option<String>,
    /// Annotation density: sparse | medium | rich (default: medium, or the
    /// note-profile document's `density`).
    #[arg(long)]
    density: Option<String>,
}

#[derive(Parser)]
struct AnnotateArgs {
    /// Input file (.epub or .txt).
    input: PathBuf,
    /// Reader background + motivation (free text, or @file to read a file).
    #[arg(long)]
    profile: Option<String>,
    /// Language the notes are written in (default: the profile's own language).
    #[arg(long)]
    note_lang: Option<String>,
    /// Note style paragraph (tone / depth / length target; free text, or @file).
    /// Injected inside the locked hard rules; defaults to the engine style.
    #[arg(long)]
    note_style: Option<String>,
    /// The service menu, comma-separated ids: terms, history, author,
    /// culture, characters, concepts, world, methods, research. What the
    /// notes should do for you; with at least one picked, --profile becomes
    /// optional. Unknown ids are ignored with a warning.
    #[arg(long)]
    note_presets: Option<String>,
    /// Reader-profile document (讀者側寫契約): a JSON file path, or inline JSON
    /// starting with `{`. Fields: purpose, anchors, presets, voice, lang,
    /// density, style — all optional; explicit flags override. See
    /// docs/READER-PROFILE.md for the schema and the standard prompt that lets
    /// the reader's own AI fill it.
    #[arg(long)]
    note_profile: Option<String>,
    /// Cognitive anchors (認知錨), comma-separated short labels of what the
    /// reader already knows (e.g. "軟體工程師,讀過《國富論》"). Notes bridge
    /// new concepts FROM these; never quoted in the notes.
    #[arg(long)]
    note_anchors: Option<String>,
    /// Explanation level: beginner (入門白話: everyday language + examples,
    /// no jargon assumed) | general (default) | insider (內行: only what an
    /// insider could not easily look up).
    #[arg(long)]
    note_level: Option<String>,
    /// Note voice register: study (書齋, default) | companion (陪讀).
    #[arg(long)]
    note_voice: Option<String>,
    /// Annotation density: sparse | medium | rich (default: medium, or the
    /// note-profile document's `density`).
    #[arg(long)]
    density: Option<String>,
    #[arg(long, default_value = "mock")]
    provider: String,
    #[arg(long, default_value = "mock")]
    model: String,
    /// Override provider base URL (OpenAI-compatible endpoints / local servers).
    #[arg(long)]
    base_url: Option<String>,
    /// Output path. Defaults to <input>.annotated.<ext>.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Job/cache DB path (resume key). Defaults to <output>.etjob.
    #[arg(long)]
    job: Option<PathBuf>,
    /// Re-render output from the cache only — no LLM calls, zero cost.
    #[arg(long)]
    cache_only: bool,
}

#[derive(Parser)]
struct EstimateArgs {
    input: PathBuf,
    #[arg(long)]
    to: String,
    #[arg(long, default_value = "sentence")]
    level: String,
    /// Model to price against. Defaults to the one saved in Settings, so the
    /// number quoted is for the model the run will actually use — a fixed
    /// default meant estimating on a cheap model and translating on an
    /// expensive one, which is a 3.5x surprise, not an estimate.
    #[arg(long)]
    model: Option<String>,
    /// Include the annotation passes (read whole book + sparse notes + review)
    /// in the estimate.
    #[arg(long)]
    annotate: bool,
    /// Annotation density (affects the note-volume guess): sparse | medium | rich.
    #[arg(long, default_value = "medium")]
    density: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();

    // ^C leaves the job cache consistent; tell the user resuming is free.
    tokio::spawn(async {
        if tokio::signal::ctrl_c().await.is_ok() {
            // The interactive session may own the screen; give it back before
            // writing anything, or the message lands in the alternate screen
            // and disappears with it.
            tui::restore();
            eprintln!(
                "\ninterrupted — progress is saved. \
                 Re-run the same command to resume; cached segments are never re-billed."
            );
            std::process::exit(130);
        }
    });

    let cli = Cli::parse();
    let Some(cmd) = cli.cmd else {
        // No subcommand: a human at a terminal gets the interactive session;
        // anything else gets usage on stderr, as a CLI should.
        if tui::should_launch() {
            return tui::run().await?.into_exit();
        }
        use clap::CommandFactory;
        Cli::command().print_help()?;
        println!();
        std::process::exit(2);
    };
    match cmd {
        Command::Estimate(a) => run_estimate(a, cli.json),
        Command::Translate(a) => run_translate(a, cli.json, Ui::Cli).await?.into_exit(),
        Command::Annotate(a) => run_annotate(a, cli.json).await?.into_exit(),
        Command::Mcp(a) => match a.cmd {
            None => mcp::serve().await,
            Some(McpCommand::Install) => {
                for l in mcp::register(true)? {
                    println!("{l}");
                }
                Ok(())
            }
            Some(McpCommand::Uninstall) => {
                for l in mcp::register(false)? {
                    println!("{l}");
                }
                Ok(())
            }
        },
    }
}

/// How a run ended, separate from whether it errored.
///
/// A run that reaches the end having failed every segment still writes an
/// output file and a resumable cache, so it is not an `Err` — but it is not a
/// success either, and a script that only checks the exit code must not read it
/// as one. The JSON summary has carried `units_failed` all along; this makes the
/// exit code agree with it.
#[must_use]
pub struct RunOutcome {
    pub units_failed: usize,
}

impl RunOutcome {
    fn into_exit(self) -> Result<()> {
        if self.units_failed > 0 {
            use std::io::Write;
            let _ = std::io::stdout().flush();
            let _ = std::io::stderr().flush();
            std::process::exit(1);
        }
        Ok(())
    }
}

pub(crate) fn safe_filename_component(label: &str) -> String {
    let mut slug = String::new();
    let mut pending_separator = false;
    for ch in label.chars() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            if pending_separator && !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
            pending_separator = false;
            slug.push(ch);
        } else if ch.is_whitespace() {
            pending_separator = true;
        }
        // Path separators, dots, controls, bidi formatting, and punctuation are
        // deliberately omitted: this value becomes exactly one filename part.
    }
    let trimmed = slug.trim_matches(['-', '_']);
    if trimmed.is_empty() {
        "translated".into()
    } else {
        trimmed.to_string()
    }
}

fn default_output(input: &Path, lang: &str) -> PathBuf {
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("out");
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
    let lang_slug = safe_filename_component(lang);
    input.with_file_name(format!("{stem}.{lang_slug}.{ext}"))
}

fn human_path(path: &Path) -> String {
    tui::sanitize_plain(&path.to_string_lossy())
}

fn resolved_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return std::fs::canonicalize(path)
            .with_context(|| format!("failed to resolve {}", human_path(path)));
    }
    // `Path::new("book.epub").parent()` is `Some("")`, and canonicalizing ""
    // fails — a bare filename means the current directory.
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let file = path
        .file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", human_path(path)))?;
    Ok(std::fs::canonicalize(parent)
        .with_context(|| format!("failed to resolve parent of {}", human_path(path)))?
        .join(file))
}

fn same_file(a: &Path, b: &Path) -> Result<bool> {
    let same_path = resolved_path(a)? == resolved_path(b)?;
    if same_path {
        return Ok(true);
    }
    #[cfg(unix)]
    if a.exists() && b.exists() {
        use std::os::unix::fs::MetadataExt;
        let ma = std::fs::metadata(a)?;
        let mb = std::fs::metadata(b)?;
        return Ok(ma.dev() == mb.dev() && ma.ino() == mb.ino());
    }
    Ok(false)
}

fn ensure_distinct_run_paths(input: &Path, output: &Path, job: &Path) -> Result<()> {
    for (name, path) in [("output", output), ("job", job)] {
        if std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(anyhow!(
                "{name} path is a symlink ({}); refusing to follow it",
                human_path(path)
            ));
        }
    }
    for (left_name, left, right_name, right) in [
        ("input", input, "output", output),
        ("input", input, "job", job),
        ("output", output, "job", job),
    ] {
        if same_file(left, right)? {
            return Err(anyhow!(
                "{left_name} and {right_name} resolve to the same file ({}); refusing to overwrite source data or the resume cache",
                human_path(left)
            ));
        }
    }
    Ok(())
}

fn resolve_api_key(provider: ProviderKind) -> Option<String> {
    let allow_ambient = std::env::var_os("TRANSLATUS_NO_AMBIENT_CREDENTIALS").is_none();
    resolve_api_key_with(
        provider,
        None,
        allow_ambient,
        |service| et_core::secrets::get_key(service).ok().flatten(),
        |name| std::env::var(name).ok(),
    )
}

fn resolve_api_key_with<K, E>(
    provider: ProviderKind,
    explicit: Option<String>,
    allow_ambient: bool,
    keychain: K,
    env: E,
) -> Option<String>
where
    K: FnOnce(&str) -> Option<String>,
    E: FnOnce(&str) -> Option<String>,
{
    if explicit.is_some() {
        return explicit;
    }
    if !allow_ambient {
        // Set only by the MCP parent after an operator-configured endpoint URL
        // exactly matches the caller request. It is never a general ambient
        // provider credential.
        return env("TRANSLATUS_SCOPED_ENDPOINT_TOKEN");
    }
    let (service, env_name) = match provider {
        ProviderKind::OpenAi => ("openai", "OPENAI_API_KEY"),
        ProviderKind::Anthropic => ("anthropic", "ANTHROPIC_API_KEY"),
        _ => return None,
    };
    // Env beats keychain: an env var on the invocation is a per-run choice,
    // while the keychain holds whatever was saved last. The other order made
    // `OPENAI_API_KEY=<fresh sidecar token> translatus …` silently use a stale
    // stored token and fail every call with nothing to go on.
    env(env_name).or_else(|| keychain(service))
}

/// Resolve a text-or-`@file` flag value (used by `--profile` / `--note-style`).
fn resolve_text_arg(raw: &str, what: &str) -> Result<String> {
    let text = if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path).with_context(|| format!("failed to read {what} {path}"))?
    } else {
        raw.to_string()
    };
    Ok(text.trim().to_string())
}

/// Resolve a `--profile` value: literal text, or `@path` to read a file.
fn resolve_profile(raw: &str) -> Result<String> {
    let profile = resolve_text_arg(raw, "profile")?;
    if profile.is_empty() {
        return Err(anyhow!("reader profile is empty"));
    }
    Ok(profile)
}

/// Resolve a `--note-profile` value into the reader-profile contract document:
/// inline JSON (starts with `{`) or a JSON file path. UNTRUSTED input — every
/// field still goes through the same normalisation as the plain flags.
fn resolve_note_profile(raw: &str) -> Result<et_core::config::ReaderProfile> {
    let text = if raw.trim_start().starts_with('{') {
        raw.to_string()
    } else {
        std::fs::read_to_string(raw)
            .with_context(|| format!("failed to read note profile {raw}"))?
    };
    et_core::config::ReaderProfile::from_json(&text).map_err(|e| anyhow!(e))
}

/// Build the annotation config for `translate --annotate` / `annotate`.
/// Sources merge as: explicit flag > reader-profile document > default.
/// `fallback_lang` implements AN-007: notes follow the translation target when
/// translating; annotate-only leaves `None` (= follow the profile's language).
#[allow(clippy::too_many_arguments)]
fn build_annotation_config(
    profile: &Option<String>,
    note_profile: &Option<String>,
    note_level: &Option<String>,
    note_anchors: &Option<String>,
    note_voice: &Option<String>,
    note_lang: &Option<String>,
    note_style: &Option<String>,
    note_presets: &Option<String>,
    density: &Option<String>,
    fallback_lang: Option<&str>,
) -> Result<AnnotationConfig> {
    let doc = match note_profile.as_deref() {
        Some(raw) => resolve_note_profile(raw)?,
        None => et_core::config::ReaderProfile::default(),
    };
    let level: et_core::config::ExplainLevel = note_level
        .as_deref()
        .or(doc.level.as_deref())
        .map(str::parse)
        .transpose()
        .map_err(|e: String| anyhow!("{e}"))?
        .unwrap_or_default();
    // Preset service ids: comma-separated, case-insensitive. Unknown ids are
    // ignored with a warning (the engine only ever consumes the canonical set).
    let presets: Vec<String> = match note_presets.as_deref() {
        Some(s) => s
            .split(',')
            .map(|x| x.trim().to_ascii_lowercase())
            .filter(|x| !x.is_empty())
            .collect(),
        None => doc
            .presets
            .iter()
            .map(|x| x.trim().to_ascii_lowercase())
            .filter(|x| !x.is_empty())
            .collect(),
    };
    for unk in et_core::annotate::prompt::unknown_presets(&presets) {
        let known: Vec<&str> = et_core::annotate::prompt::PRESETS
            .iter()
            .map(|(id, _)| *id)
            .collect();
        eprintln!(
            "warning: unknown note service `{unk}` ignored (known: {})",
            known.join(", ")
        );
    }
    let services_valid = !et_core::annotate::prompt::canonical_presets(&presets).is_empty();
    // Purpose: explicit --profile wins; else the document's `purpose`; with at
    // least one service ticked the free text is optional (picking beats
    // articulating — the prompts substitute an honest fallback line).
    let reader_profile = match profile.as_deref() {
        Some(raw) => resolve_profile(raw)?,
        None => {
            let from_doc = doc
                .purpose
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            match from_doc {
                Some(p) => p,
                None if services_valid => String::new(),
                None => {
                    return Err(anyhow!(
                        "notes need a reason to exist: pass --profile (text or @file), \
                         a --note-profile document with `purpose`, or pick at least one \
                         --note-presets service (terms, history, author, culture, \
                         characters, concepts, world, methods, research)"
                    ))
                }
            }
        }
    };
    let density: Density = density
        .as_deref()
        .or(doc.density.as_deref())
        .unwrap_or("medium")
        .parse()
        .map_err(|e: String| anyhow!("{e}"))?;
    let voice: et_core::config::NoteVoice = note_voice
        .as_deref()
        .or(doc.voice.as_deref())
        .map(str::parse)
        .transpose()
        .map_err(|e: String| anyhow!("{e}"))?
        .unwrap_or_default();
    let style = match note_style.as_deref() {
        Some(raw) => Some(resolve_text_arg(raw, "note style")?).filter(|s| !s.is_empty()),
        None => doc
            .style
            .clone()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    };
    let anchors: Vec<String> = match note_anchors.as_deref() {
        Some(s) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        None => doc.anchors.clone(),
    };
    Ok(AnnotationConfig {
        reader_profile,
        level,
        anchors,
        voice,
        lang: note_lang
            .clone()
            .or_else(|| doc.lang.clone().filter(|s| !s.trim().is_empty()))
            .or_else(|| fallback_lang.map(str::to_string)),
        density,
        style,
        presets,
    })
}

/// Shared progress printer for the annotation phases.
fn report_annotation_phase(p: &translate::RunProgress, json_out: bool) {
    match p.phase {
        translate::Phase::AnnotatePlan => {
            if json_out {
                println!(
                    "{}",
                    json!({ "event": "annotate_plan", "total": p.total_chapters })
                );
            } else {
                eprintln!("  [pass N0] planning margin notes across the book…");
            }
        }
        translate::Phase::Annotating => {
            // Batch events carry notes; the chapter-boundary event carries counts.
            if !p.notes.is_empty() {
                // Agents consuming JSON get each freshly written note with its
                // placement (AN-014) as it lands.
                if json_out {
                    println!(
                        "{}",
                        json!({
                            "event": "notes",
                            "href": p.href,
                            "notes": p.notes.iter().map(|(_, n)| {
                                json!({ "pos": n.pos, "note": n.text })
                            }).collect::<Vec<_>>(),
                        })
                    );
                }
                return;
            }
            if json_out {
                println!(
                    "{}",
                    json!({
                        "event": "annotating",
                        "chapter": p.chapter_index + 1,
                        "total": p.total_chapters,
                        "href": p.href,
                        "notes_written": p.units_translated,
                        "units_failed": p.units_failed,
                    })
                );
            } else {
                eprintln!(
                    "  [notes {}/{}] {} — {} written",
                    p.chapter_index + 1,
                    p.total_chapters,
                    p.href,
                    p.units_translated
                );
            }
        }
        translate::Phase::AnnotateReview => {
            if json_out {
                println!("{}", json!({ "event": "annotate_review" }));
            } else {
                eprintln!("  [pass N2] reviewing all notes as one book…");
            }
        }
        _ => {}
    }
}

/// Estimate numbers shared by `estimate` and the pre-run summary line that
/// human-mode `translate` / `annotate` print before spending tokens.
struct EstimateNums {
    tokens_in: u64,
    tokens_out: u64,
    cost: f64,
    /// (tokens_in, tokens_out, cost) of the annotation passes, when requested.
    annotation: Option<(u64, u64, f64)>,
}

fn estimate_numbers(
    book: &et_core::document::Book,
    model: &str,
    level: Level,
    annotate_density: Option<Density>,
) -> EstimateNums {
    let source = book.est_source_tokens() as u64;
    // How many times the pipeline actually reads and rewrites the text.
    // Sentence level is one pass. Expert reads the whole book to build the
    // glossary, drafts, reflects against the source, then reads it again for
    // the whole-book consistency check: four reads, two rewrites. Leaving this
    // out priced a four-pass run as a one-pass run.
    let (in_passes, out_passes) = match level {
        Level::Expert => (4u64, 2u64),
        _ => (1, 1),
    };
    let tokens_in = source * in_passes;
    // crude: output ≈ 1.1× input for CJK targets
    let tokens_out = (source as f64 * 1.1) as u64 * out_passes;
    let cost = et_core::estimate_cost_usd(model, tokens_in, tokens_out);
    let annotation = annotate_density.map(|density| {
        // Notes-out volume as a fraction of the source — a coarse density guess.
        let note_ratio = match density {
            Density::Sparse => 0.03,
            Density::Medium => 0.07,
            Density::Rich => 0.14,
        };
        let notes_out = (tokens_in as f64 * note_ratio) as u64;
        // The selection pass reads the whole book compressed (~1/3); the
        // writing pass reads only the selected paragraphs plus context (≈3×
        // the note volume); N2 re-reads every note with book-wide context and
        // rewrites a fraction of them (≈2× the note volume).
        let anno_in = tokens_in / 3 + notes_out * 5;
        let anno_out = notes_out + notes_out / 3;
        (
            anno_in,
            anno_out,
            et_core::estimate_cost_usd(model, anno_in, anno_out),
        )
    });
    EstimateNums {
        tokens_in,
        tokens_out,
        cost,
        annotation,
    }
}

/// One-line cost preview printed (stderr) before a human-mode run starts.
/// Mirrors the desktop flow, which always shows an estimate first. No
/// confirmation prompt — CLI convention is to print and proceed.
fn print_run_estimate(est: &EstimateNums, model: &str) {
    let total_cost = est.cost + est.annotation.map(|(_, _, c)| c).unwrap_or(0.0);
    let (mut tin, mut tout) = (est.tokens_in, est.tokens_out);
    if let Some((ai, ao, _)) = est.annotation {
        tin += ai;
        tout += ao;
    }
    if total_cost > 0.0 {
        eprintln!(
            "estimate: ~{tin} tokens in / ~{tout} out ≈ {} ({model})",
            fmt_usd(total_cost)
        );
    } else {
        eprintln!("estimate: ~{tin} tokens in / ~{tout} out ({model}; no price data — see `translatus estimate`)");
    }
}

/// The model an estimate prices against when the caller did not name one:
/// whatever the user configured, falling back to the documented default only
/// when there is no configuration to read.
fn estimate_model_default() -> String {
    let saved = tui::store::load().api.model;
    if saved.trim().is_empty() {
        "gpt-5.4-mini".to_string()
    } else {
        saved
    }
}

fn run_estimate(a: EstimateArgs, json_out: bool) -> Result<()> {
    let (book, _doc) = format::extract(&a.input).with_context(|| "failed to parse input")?;
    let density = if a.annotate {
        Some(
            a.density
                .parse::<Density>()
                .map_err(|e: String| anyhow!("{e}"))?,
        )
    } else {
        None
    };
    let model = a.model.clone().unwrap_or_else(estimate_model_default);
    let level: Level = a.level.parse().map_err(|e: String| anyhow!("{e}"))?;
    let est = estimate_numbers(&book, &model, level, density);
    if !json_out {
        println!(
            "{} → {} ({}): {}, {}",
            human_path(&a.input),
            a.to,
            a.level,
            n_of(book.chapters.len(), "chapter"),
            n_of(book.total_segments(), "segment"),
        );
        println!(
            "  translation ~{} tokens in / ~{} out ≈ {} ({})",
            est.tokens_in,
            est.tokens_out,
            fmt_usd(est.cost),
            model
        );
        if let Some((ai, ao, ac)) = est.annotation {
            println!(
                "  margin notes ({}) ~{ai} tokens in / ~{ao} out ≈ {}",
                a.density,
                fmt_usd(ac)
            );
            println!("  total ≈ {}", fmt_usd(est.cost + ac));
        }
        for c in book.apparatus_chapters() {
            println!(
                "  skipped (publisher apparatus, never sent to a model): {}",
                c.href
            );
        }
        println!("(rough numbers; actuals depend on the book and, for notes, the reader profile)");
        return Ok(());
    }
    let mut out = json!({
        "input": a.input.display().to_string(),
        "to": a.to,
        "level": a.level,
        "chapters": book.chapters.len(),
        "segments": book.total_segments(),
        "est_tokens_in": est.tokens_in,
        "est_tokens_out": est.tokens_out,
        "est_cost_usd": est.cost,
        // What the run will not send to a model, and why the counts above are
        // smaller than the book. Named rather than merely counted: a silently
        // skipped page is indistinguishable from a bug.
        "skipped_apparatus": book
            .apparatus_chapters()
            .iter()
            .map(|c| c.href.clone())
            .collect::<Vec<_>>(),
        "model": model,
    });
    if let Some((anno_in, anno_out, anno_cost)) = est.annotation {
        out["annotation"] = json!({
            "density": a.density,
            "est_tokens_in": anno_in,
            "est_tokens_out": anno_out,
            "est_cost_usd": anno_cost,
            "note": "rough estimate: the selection pass reads the whole book compressed, and note volume is locked by a hard per-chapter cap; actual spots depend on the book and the reader profile",
        });
        out["est_total_cost_usd"] = json!(est.cost + anno_cost);
    }
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Fail fast — before parsing the book — when a hosted provider has no key at
/// all, with a three-way setup card on first run.
fn require_credentials(
    provider: ProviderKind,
    api_key: &Option<String>,
    base_url: &Option<String>,
) -> Result<()> {
    // A provider we cannot speak to at all is a better error than "no API key
    // for it" — the key would not have helped.
    if let Some(why) = et_core::llm::unsupported_reason(provider) {
        anyhow::bail!(why);
    }
    let needs_key = matches!(provider, ProviderKind::OpenAi);
    // A custom base URL means a local/self-hosted endpoint (sidecar, proxy,
    // LM Studio…) which may be keyless on purpose.
    if !needs_key || api_key.is_some() || base_url.is_some() {
        return Ok(());
    }
    let env_var = "OPENAI_API_KEY";
    let lines = [
        format!("no API key found for this provider (checked the OS keychain and ${env_var})."),
        String::new(),
        "Three ways to connect a model:".into(),
        String::new(),
        "  subscription   use Codex/ChatGPT or Claude Code via the local sidecar:".into(),
        "                   cd apps/subscription-kit && npm install && npm start".into(),
        "                   then add: --base-url http://127.0.0.1:8765/v1".into(),
        format!("  API key        export {env_var}=...   (or save it in the OS keychain)"),
        "  ollama         free and fully local:  --provider ollama --model <name>".into(),
        String::new(),
        "`--provider mock` runs the whole pipeline offline for free (a dry run).".into(),
        "Details: `translatus --help` or docs/GUIDE.md.".into(),
    ];
    Err(anyhow!(lines.join("\n")))
}

/// Money formatting for human output: dollars-and-cents for real amounts,
/// four decimals for sub-cent estimates, plain zero for free runs.
fn fmt_usd(v: f64) -> String {
    if v == 0.0 {
        "US$0.00".into()
    } else if v < 0.01 {
        "under US$0.01".into()
    } else {
        format!("US${v:.2}")
    }
}

/// "1 chapter" / "3 chapters" — human output only.
fn n_of(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{n} {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Human-mode notice that mock runs are dry runs, so nobody mistakes the
/// output for a real translation.
fn print_mock_notice(provider: ProviderKind, json_out: bool) {
    if !json_out && matches!(provider, ProviderKind::Mock) {
        eprintln!(
            "note: using the built-in mock provider — an offline dry run that checks \
             formatting, not a real translation. See `translatus --help` to connect a model."
        );
    }
}

async fn run_translate(a: TranslateArgs, json_out: bool, ui: Ui) -> Result<RunOutcome> {
    let level: Level = a.level.parse().map_err(|e| anyhow!("{e}"))?;
    let mode: OutputMode = a.mode.parse().map_err(|e| anyhow!("{e}"))?;
    let provider_kind: ProviderKind = a.provider.parse().map_err(|e| anyhow!("{e}"))?;

    let output = a
        .output
        .clone()
        .unwrap_or_else(|| default_output(&a.input, &a.to));
    let job_path = a
        .job
        .clone()
        .unwrap_or_else(|| output.with_extension("etjob"));
    ensure_distinct_run_paths(&a.input, &output, &job_path)?;

    let mut cfg = TranslateConfig::new(&a.to);
    cfg.level = level;
    cfg.output_mode = mode;
    cfg.provider = provider_kind;
    cfg.model = a.model.clone();
    cfg.custom_prompt = a.prompt.clone();
    cfg.concurrency = a.concurrency.max(1);
    if a.annotate {
        // AN-007: notes default to the translation's target language.
        cfg.annotations = Some(build_annotation_config(
            &a.profile,
            &a.note_profile,
            &a.note_level,
            &a.note_anchors,
            &a.note_voice,
            &a.note_lang,
            &a.note_style,
            &a.note_presets,
            &a.density,
            Some(&a.to),
        )?);
    }
    let sig = cfg.cache_signature();
    let note_lang = cfg.annotations.as_ref().and_then(|an| an.lang.clone());

    // Resolve API key: explicit flag > OS keychain > environment.
    let api_key = resolve_api_key(provider_kind);

    let (mut book, doc) = format::extract(&a.input).with_context(|| "failed to parse input")?;
    let store = job::JobStore::open(&job_path)?;

    // Cache-only: re-render from the cache with no LLM calls (zero cost).
    if a.cache_only {
        let sig = store.config_sig()?.unwrap_or_else(|| {
            // The most common way here: the user re-ran with a different
            // --output, which derives a NEW (empty) job next to it.
            eprintln!(
                "warning: {} has no run history — if you changed --output, point --job at the original .etjob from the first run",
                human_path(&job_path)
            );
            sig.clone()
        });
        // Annotations: prefer the signature the notes were written under (the
        // job meta), falling back to the current flags.
        let anno_sig = match store.get_meta("anno_sig")? {
            Some(s) => Some(s),
            None => cfg.annotation_signature(),
        };
        let rs = translate::render_from_cache(&mut book, &store, Some(&sig), anno_sig.as_deref())?;
        if rs.missing > 0 {
            eprintln!(
                "warning: {}/{} segments not cached; output uses source",
                rs.missing, rs.total_segments
            );
        }
        format::write(&doc, &book, &output, mode, &a.to, note_lang.as_deref())
            .with_context(|| "failed to write output")?;
        if json_out {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "event": "done", "cache_only": true,
                    "output": output.display().to_string(), "job": job_path.display().to_string(),
                    "segments": rs.total_segments, "restored_from_cache": rs.restored_from_cache,
                    "missing": rs.missing,
                    "note_segments_prefilled": rs.note_segments_prefilled,
                    "notes_in_output": rs.notes_in_output,
                    "tokens_in": 0, "tokens_out": 0, "est_cost_usd": 0.0,
                }))?
            );
        } else {
            println!("re-exported from cache: {}", human_path(&output));
            println!(
                "  {}/{} segments from cache · {} missing · {} notes in output · zero LLM calls, US$0.00",
                rs.restored_from_cache, rs.total_segments, rs.missing, rs.notes_in_output
            );
        }
        return Ok(RunOutcome { units_failed: 0 });
    }

    store.set_meta("target_lang", &a.to)?;
    store.set_meta("config_sig", &sig)?;

    require_credentials(provider_kind, &api_key, &a.base_url)?;
    let provider = et_core::llm::Provider::from_config(&cfg, api_key, a.base_url.clone())?;
    let total_chapters = book.chapters.len();
    // Interactive runs paint a live board instead of appending a line per
    // chapter. Titles and sizes are snapshotted here so the board can render
    // the whole table of contents up front, greyed, before any work starts.
    let expert = cfg.level == Level::Expert;
    let mut board = (ui == Ui::Tui).then(|| {
        tui::board::Board::new(
            &a.input
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            book.chapters
                .iter()
                .enumerate()
                .map(|(i, ch)| {
                    (
                        ch.title
                            .clone()
                            .filter(|t| !t.trim().is_empty())
                            .unwrap_or_else(|| {
                                tui::i18n::tr1("run.chapter.title", &(i + 1).to_string())
                            }),
                        ch.segments.iter().map(|s| s.source.chars().count()).sum(),
                    )
                })
                .collect(),
        )
    });
    if !json_out && ui == Ui::Cli {
        eprintln!(
            "translating {} → {} ({:?}); {}, {}",
            human_path(&a.input),
            a.to,
            level,
            n_of(total_chapters, "chapter"),
            n_of(book.total_segments(), "segment"),
        );
        print_mock_notice(provider_kind, json_out);
        if !matches!(provider_kind, ProviderKind::Mock) {
            let density = cfg.annotations.as_ref().map(|an| an.density);
            print_run_estimate(
                &estimate_numbers(&book, &cfg.model, cfg.level, density),
                &cfg.model,
            );
        }
    }

    let mut chapter_started = std::time::Instant::now();
    if let Some(b) = board.as_mut() {
        if expert {
            b.chapter_granular();
        }
        let _ = b.start(0);
    }
    let summary = translate::run(&provider, &cfg, &mut book, &store, &sig, |p| {
        // Board mode consumes every event: a finished chapter advances the
        // table, anything else just spins the row that is currently working.
        if let Some(b) = board.as_mut() {
            match p.phase {
                translate::Phase::Translating => {
                    let took = chapter_started.elapsed();
                    chapter_started = std::time::Instant::now();
                    let _ = b.finish(p.chapter_index, took, p.units_failed == 0);
                    let _ = b.start(p.chapter_index + 1);
                }
                // Whole-book passes: say what they are, and restart the chapter
                // clock after them. They are a fixed cost of the run, not work
                // done on chapter 1 — charging them there made the first row
                // read "11 chars, 7m" and threw the ETA off by two orders of
                // magnitude.
                translate::Phase::Prescan => {
                    chapter_started = std::time::Instant::now();
                    let _ = b.prep(tui::i18n::tr("run.prescan"));
                }
                translate::Phase::AnnotatePlan => {
                    chapter_started = std::time::Instant::now();
                    let _ = b.prep(tui::i18n::tr("run.noteplan"));
                }
                _ => {
                    let _ = b.tick();
                }
            }
            return;
        }
        match p.phase {
            translate::Phase::Batch => {} // desktop-only batch streaming; the CLI reports per chapter
            translate::Phase::AnnotatePlan
            | translate::Phase::Annotating
            | translate::Phase::AnnotateReview => report_annotation_phase(&p, json_out),
            translate::Phase::Prescan => {
                if json_out {
                    println!(
                        "{}",
                        json!({ "event": "prescan", "total": p.total_chapters })
                    );
                } else {
                    eprintln!("  [pass 0] building glossary / style guide…");
                }
            }
            translate::Phase::Translating => {
                if json_out {
                    println!(
                        "{}",
                        json!({
                            "event": "chapter",
                            "chapter": p.chapter_index + 1,
                            "total": p.total_chapters,
                            "href": p.href,
                            "units_translated": p.units_translated,
                            "units_failed": p.units_failed,
                        })
                    );
                } else {
                    eprintln!(
                        "  [{}/{}] {} — {} translated, {} failed",
                        p.chapter_index + 1,
                        p.total_chapters,
                        p.href,
                        p.units_translated,
                        p.units_failed
                    );
                }
            }
            translate::Phase::Consistency => {
                if !json_out {
                    eprintln!("  [pass 3] whole-book consistency check…");
                }
            }
        }
    })
    .await
    .with_context(|| "translation failed")?;

    format::write(&doc, &book, &output, mode, &a.to, note_lang.as_deref())
        .with_context(|| "failed to write output")?;

    let cost = et_core::estimate_cost_usd(&cfg.model, summary.tokens_in, summary.tokens_out);
    let mut result = json!({
        "event": "done",
        "output": output.display().to_string(),
        "job": job_path.display().to_string(),
        "chapters": total_chapters,
        "segments": book.total_segments(),
        "restored_from_cache": summary.restored_from_cache,
        "units_translated": summary.units_translated,
        "units_failed": summary.units_failed,
        "tokens_in": summary.tokens_in,
        "tokens_out": summary.tokens_out,
        "est_cost_usd": cost,
        "glossary_size": summary.glossary_size,
        "inconsistencies": summary.inconsistencies,
    });
    if let Some(e) = &summary.sample_error {
        result["sample_error"] = json!(e);
    }
    if a.annotate {
        result["notes_written"] = json!(summary.notes_written);
        result["notes_dropped"] = json!(summary.notes_dropped);
        result["notes_edited"] = json!(summary.notes_edited);
        result["notes_restored_from_cache"] = json!(summary.notes_restored_from_cache);
    }
    if json_out {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(RunOutcome { units_failed: 0 });
    }

    if let Some(b) = board.as_ref() {
        // Drop back to the real screen before writing anything worth keeping,
        // or the summary dies with the alternate screen.
        tui::restore();
        for line in b.summary(&output, Some(cost)) {
            println!("{line}");
        }
        println!(
            "  {}",
            tui::i18n::tr(if expert {
                "run.resume.expert"
            } else {
                "run.resume"
            })
        );
        return Ok(RunOutcome { units_failed: 0 });
    }

    // Human-mode wrap-up: same shape as the desktop completion page.
    println!("done: {}", human_path(&output));
    println!(
        "  {} · {} — {} translated, {} failed, {} from cache",
        n_of(total_chapters, "chapter"),
        n_of(book.total_segments(), "segment"),
        summary.units_translated,
        summary.units_failed,
        summary.restored_from_cache,
    );
    if cost > 0.0 {
        println!(
            "  tokens {} in / {} out ≈ {} ({})",
            summary.tokens_in,
            summary.tokens_out,
            fmt_usd(cost),
            cfg.model
        );
    } else {
        println!(
            "  tokens {} in / {} out ({})",
            summary.tokens_in, summary.tokens_out, cfg.model
        );
    }
    if summary.glossary_size > 0 || !summary.inconsistencies.is_empty() {
        println!(
            "  glossary {} terms · {} consistency flags",
            summary.glossary_size,
            summary.inconsistencies.len()
        );
    }
    if a.annotate {
        println!(
            "  notes {} written · review edited {}, dropped {} · {} from cache",
            summary.notes_written,
            summary.notes_edited,
            summary.notes_dropped,
            summary.notes_restored_from_cache,
        );
    }
    if summary.units_failed > 0 {
        println!(
            "  ⚠ {} segments kept the original — re-run the same command to fill only those",
            summary.units_failed
        );
        if let Some(e) = &summary.sample_error {
            println!("    first error: {}", tui::term::sanitize_plain(e));
        }
        if summary.units_translated == 0 && summary.tokens_out == 0 {
            println!(
                "    every call failed — {} looks unreachable, or it rejected the key.\n    Check --base-url and your key. Using a subscription? Make sure the sidecar\n    is running and the token matches the one it printed at startup.",
                a.base_url.as_deref().unwrap_or("the default endpoint")
            );
        }
    }
    println!(
        "job cache: {} (re-running the same command resumes; cached segments are never re-billed)",
        human_path(&job_path)
    );
    Ok(RunOutcome {
        units_failed: summary.units_failed,
    })
}

/// `translatus annotate` — margin notes only, no translation. The output keeps the source
/// text byte-faithful and inserts the notes after their paragraphs.
async fn run_annotate(a: AnnotateArgs, json_out: bool) -> Result<RunOutcome> {
    let provider_kind: ProviderKind = a.provider.parse().map_err(|e| anyhow!("{e}"))?;

    let output = a.output.clone().unwrap_or_else(|| {
        let ext = a
            .input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("out");
        let stem = a
            .input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("book");
        a.input.with_file_name(format!("{stem}.annotated.{ext}"))
    });
    let job_path = a
        .job
        .clone()
        .unwrap_or_else(|| output.with_extension("etjob"));
    ensure_distinct_run_paths(&a.input, &output, &job_path)?;

    // The target language is irrelevant here (no translation runs); notes
    // follow --note-lang or the profile's own language (AN-007).
    let mut cfg = TranslateConfig::new("原文");
    cfg.output_mode = OutputMode::Replace;
    cfg.provider = provider_kind;
    cfg.model = a.model.clone();

    let (mut book, doc) = format::extract(&a.input).with_context(|| "failed to parse input")?;
    let store = job::JobStore::open(&job_path)?;

    if a.cache_only {
        let anno_sig = store.get_meta("anno_sig")?.ok_or_else(|| {
            anyhow!(
                "this job has no annotation cache (missing anno_sig), so there is \
                 nothing to re-render for free — run without --cache-only first"
            )
        })?;
        let rs = translate::render_from_cache(&mut book, &store, None, Some(&anno_sig))?;
        let note_lang = a.note_lang.clone();
        format::write(
            &doc,
            &book,
            &output,
            OutputMode::Replace,
            "原文",
            note_lang.as_deref(),
        )
        .with_context(|| "failed to write output")?;
        if json_out {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "event": "done", "cache_only": true,
                    "output": output.display().to_string(), "job": job_path.display().to_string(),
                    "segments": rs.total_segments,
                    "note_segments_prefilled": rs.note_segments_prefilled,
                    "notes_in_output": rs.notes_in_output,
                    "tokens_in": 0, "tokens_out": 0, "est_cost_usd": 0.0,
                }))?
            );
        } else {
            println!("re-exported from cache: {}", human_path(&output));
            println!(
                "  {} notes in output across {} segments · zero LLM calls, US$0.00",
                rs.notes_in_output, rs.total_segments
            );
        }
        return Ok(RunOutcome { units_failed: 0 });
    }

    cfg.annotations = Some(build_annotation_config(
        &a.profile,
        &a.note_profile,
        &a.note_level,
        &a.note_anchors,
        &a.note_voice,
        &a.note_lang,
        &a.note_style,
        &a.note_presets,
        &a.density,
        None,
    )?);
    let note_lang = cfg.annotations.as_ref().and_then(|an| an.lang.clone());

    let api_key = resolve_api_key(provider_kind);
    require_credentials(provider_kind, &api_key, &a.base_url)?;
    let provider = et_core::llm::Provider::from_config(&cfg, api_key, a.base_url.clone())?;

    if !json_out {
        eprintln!(
            "annotating {}; {}, {}",
            human_path(&a.input),
            n_of(book.chapters.len(), "chapter"),
            n_of(book.total_segments(), "segment"),
        );
        print_mock_notice(provider_kind, json_out);
        if !matches!(provider_kind, ProviderKind::Mock) {
            let density = cfg.annotations.as_ref().map(|an| an.density);
            if let Some((ai, ao, ac)) =
                estimate_numbers(&book, &cfg.model, cfg.level, density).annotation
            {
                if ac > 0.0 {
                    eprintln!(
                        "estimate: ~{ai} tokens in / ~{ao} out ≈ {} ({})",
                        fmt_usd(ac),
                        cfg.model
                    );
                } else {
                    eprintln!(
                        "estimate: ~{ai} tokens in / ~{ao} out ({}; no price data)",
                        cfg.model
                    );
                }
            }
        }
    }

    let summary = translate::annotate_only(&provider, &cfg, &mut book, &store, |p| {
        report_annotation_phase(&p, json_out)
    })
    .await
    .with_context(|| "annotation failed")?;

    format::write(
        &doc,
        &book,
        &output,
        OutputMode::Replace,
        "原文",
        note_lang.as_deref(),
    )
    .with_context(|| "failed to write output")?;

    let cost = et_core::estimate_cost_usd(&cfg.model, summary.tokens_in, summary.tokens_out);
    if json_out {
        let mut result = json!({
            "event": "done",
            "output": output.display().to_string(),
            "job": job_path.display().to_string(),
            "chapters": book.chapters.len(),
            "segments": book.total_segments(),
            "notes_written": summary.notes_written,
            "notes_dropped": summary.notes_dropped,
            "notes_edited": summary.notes_edited,
            "notes_restored_from_cache": summary.notes_restored_from_cache,
            "units_failed": summary.units_failed,
            "tokens_in": summary.tokens_in,
            "tokens_out": summary.tokens_out,
            "est_cost_usd": cost,
        });
        if let Some(e) = &summary.sample_error {
            result["sample_error"] = json!(e);
        }
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(RunOutcome { units_failed: 0 });
    }

    println!("done: {}", human_path(&output));
    println!(
        "  {} · notes {} written · review edited {}, dropped {} · {} from cache",
        n_of(book.chapters.len(), "chapter"),
        summary.notes_written,
        summary.notes_edited,
        summary.notes_dropped,
        summary.notes_restored_from_cache,
    );
    // A quiet "0 written" after everything failed reads like a finished run.
    // Say what actually happened and where to look — this is the first thing
    // a user with a stopped sidecar or a wrong base_url sees.
    if summary.units_failed > 0 {
        eprintln!(
            "warning: {} failed — those spots are NOT annotated. Re-running the same command retries them at no extra cost.",
            n_of(summary.units_failed, "model call")
        );
        if let Some(e) = &summary.sample_error {
            eprintln!("  first error: {}", tui::term::sanitize_plain(e));
        }
    }
    if summary.notes_written == 0
        && summary.notes_restored_from_cache == 0
        && summary.tokens_out == 0
        && !matches!(provider_kind, ProviderKind::Mock)
    {
        eprintln!(
            "warning: the model produced nothing at all — {} looks unreachable, or it rejected every call.\n  Check --base-url and your key. Using a subscription? Start the sidecar first: apps/subscription-kit → npm start",
            a.base_url.as_deref().unwrap_or("the default endpoint")
        );
    }
    if cost > 0.0 {
        println!(
            "  tokens {} in / {} out ≈ {} ({})",
            summary.tokens_in,
            summary.tokens_out,
            fmt_usd(cost),
            cfg.model
        );
    } else {
        println!(
            "  tokens {} in / {} out ({})",
            summary.tokens_in, summary.tokens_out, cfg.model
        );
    }
    println!(
        "job cache: {} (re-running the same command resumes; notes are cached, never re-billed)",
        human_path(&job_path)
    );
    Ok(RunOutcome {
        units_failed: summary.units_failed,
    })
}

#[cfg(test)]
mod cli_safety_tests {
    use super::*;

    #[test]
    fn input_output_and_job_must_be_distinct() {
        let base = std::env::temp_dir().join(format!("translatus-paths-{}", std::process::id()));
        std::fs::create_dir_all(&base).expect("test dir");
        let input = base.join("book.txt");
        let output = base.join("book.English.txt");
        let job = base.join("book.English.etjob");
        std::fs::write(&input, "book").expect("test input");
        assert!(ensure_distinct_run_paths(&input, &output, &job).is_ok());
        assert!(ensure_distinct_run_paths(&input, &input, &job).is_err());
        assert!(ensure_distinct_run_paths(&input, &output, &output).is_err());

        #[cfg(unix)]
        {
            let alias = base.join("hard-link.txt");
            std::fs::hard_link(&input, &alias).expect("hard link");
            assert!(ensure_distinct_run_paths(&input, &alias, &job).is_err());

            let victim = base.join("victim.txt");
            let link = base.join("output-link.txt");
            std::fs::write(&victim, "sentinel").expect("victim");
            std::os::unix::fs::symlink(&victim, &link).expect("symlink");
            assert!(ensure_distinct_run_paths(&input, &link, &job).is_err());
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ambient_credentials_can_be_disabled_without_touching_keychain_or_env() {
        let resolved = resolve_api_key_with(
            ProviderKind::OpenAi,
            None,
            false,
            |_| panic!("keychain must not be read"),
            |name| {
                assert_eq!(name, "TRANSLATUS_SCOPED_ENDPOINT_TOKEN");
                None
            },
        );
        assert!(resolved.is_none());

        let explicit = resolve_api_key_with(
            ProviderKind::OpenAi,
            Some("explicit-canary".into()),
            false,
            |_| panic!("keychain must not be read"),
            |_| panic!("environment must not be read"),
        );
        assert_eq!(explicit.as_deref(), Some("explicit-canary"));
    }

    /// `OPENAI_API_KEY=… translatus …` must use that key even when the
    /// keychain holds an older one. The reverse order made a freshly printed
    /// sidecar token lose to a stale stored token: every call failed with 401
    /// and the run reported only "8 failed".
    #[test]
    fn env_key_beats_a_stale_keychain_key() {
        let resolved = resolve_api_key_with(
            ProviderKind::OpenAi,
            None,
            true,
            |_| Some("stale-keychain-token".into()),
            |name| {
                assert_eq!(name, "OPENAI_API_KEY");
                Some("fresh-env-token".into())
            },
        );
        assert_eq!(resolved.as_deref(), Some("fresh-env-token"));

        // Without the env var the keychain still works.
        let fallback = resolve_api_key_with(
            ProviderKind::OpenAi,
            None,
            true,
            |_| Some("stale-keychain-token".into()),
            |_| None,
        );
        assert_eq!(fallback.as_deref(), Some("stale-keychain-token"));
    }

    #[test]
    fn target_language_cannot_escape_the_input_directory() {
        let input = Path::new("/tmp/library/book.txt");
        for hostile in ["/../../victim", "..\\..\\victim", "..", ".", "\u{1b}]8;;x"] {
            let output = default_output(input, hostile);
            assert_eq!(output.parent(), input.parent(), "escaped with {hostile:?}");
            let name = output.file_name().unwrap().to_string_lossy();
            assert!(!name.contains('/') && !name.contains('\\'));
        }
        assert_eq!(
            default_output(input, "..").file_name().unwrap(),
            "book.translated.txt"
        );
        assert_eq!(
            default_output(input, "Traditional Chinese")
                .file_name()
                .unwrap(),
            "book.Traditional-Chinese.txt"
        );
        assert_eq!(safe_filename_component("繁體中文"), "繁體中文");
    }

    #[test]
    fn resolved_path_accepts_bare_filename() {
        // `translatus annotate book.epub` run inside the book's directory
        // produces a bare relative output name; its parent is `Some("")`.
        let resolved = resolved_path(Path::new("no-such-file.annotated.txt"))
            .expect("bare relative filename must resolve against the current directory");
        assert!(resolved.is_absolute());
        assert_eq!(
            resolved.file_name().unwrap().to_str().unwrap(),
            "no-such-file.annotated.txt"
        );
    }
}
