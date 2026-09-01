//! `translatus` with no arguments: the interactive front door.
//!
//! The flag-driven CLI stays the contract for agents and scripts; this exists
//! for the person who has an .epub and does not want to read `--help` first.
//! It collects the same options the flags do, shows what the run will cost,
//! and then calls exactly the same engine path — there is no second
//! implementation of translation hiding behind the menus.
//!
//! Design lineage and the reasoning behind the visual choices:
//! `wiki/mole-cli-ux-teardown.md`.

pub mod board;
mod flow;
pub mod i18n;
mod key;
pub(crate) mod resume;
pub(crate) mod store;
pub(crate) mod term;
pub(crate) mod theme;
mod widgets;

use anyhow::Result;
use et_core::config::Density;
use et_core::format;
use i18n::{tr, tr1, trn};

/// True when we should open the TUI instead of printing usage.
///
/// A bare `translatus` in a pipeline should behave like a normal CLI and
/// report usage on stderr, not try to paint an alternate screen into a log.
pub fn should_launch() -> bool {
    term::is_interactive()
}

/// Put the terminal back. Called by the run itself, just before it prints a
/// summary that has to outlive the alternate screen.
pub fn restore() {
    term::restore();
}

pub(crate) fn sanitize_plain(text: &str) -> String {
    term::sanitize_plain(text)
}

/// Run the interactive session. Returns when the user quits.
pub async fn run() -> Result<crate::RunOutcome> {
    let root = std::env::current_dir()?;
    // Held for the whole session, including the run: the progress board needs
    // the alternate screen to address rows. The run drops back to the real
    // screen itself once it has something worth keeping.
    let _guard = term::Terminal::enter()?;
    offer_mcp_registration()?;
    loop {
        match flow::run(&root)? {
            flow::Intent::Quit => return Ok(crate::RunOutcome { units_failed: 0 }),
            flow::Intent::Continue => {}
            flow::Intent::Estimate(c) => {
                let lines = estimate_lines(&c)?;
                widgets::notice(tr("go.cost.title"), &lines)?;
            }
            flow::Intent::Run(c) => {
                let lines = estimate_lines(&c)?;
                let gate = summarise_gate(&c)?;
                if !widgets::confirm(&lines, &gate)? {
                    continue;
                }
                // The interactive session owns the terminal, so it reports the
                // outcome itself rather than letting main() exit under it.
                //
                // Translation off with notes on is the annotate-only run, and
                // it goes through the same `annotate` path the flag interface
                // uses — not a third implementation that happens to look alike.
                let cmd = command_line(&c);
                let outcome = if c.translate {
                    // Nobody reads keys during a run, so raw mode only serves to
                    // swallow ^C. Hand it back for the duration.
                    term::keys_off();
                    let r = crate::run_translate(to_args(&c), false, crate::Ui::Tui).await;
                    term::keys_on();
                    r?
                } else {
                    // The annotate path has no progress board and prints with
                    // plain `println!`, which staircases in raw mode. It has no
                    // reason to know a TUI exists, so the terminal goes back
                    // before it starts rather than teaching it about ours.
                    restore();
                    crate::run_annotate(to_annotate_args(&c), false).await?
                };
                // The flag form, in scrollback where it can be copied. The
                // confirm screen shows it too, but that one is truncated to the
                // width of a box — which makes the "graduate to the CLI" promise
                // unkeepable exactly when someone wants to keep it.
                println!("\n{}", tr1("go.samecmd", &cmd));
                return Ok(outcome);
            }
        }
    }
}

/// Offer once, on the first interactive run, to register the MCP server with
/// the agent clients already on this machine.
///
/// Why here and not in the installer: `brew install` is non-interactive, so a
/// package-manager hook could only write into Claude's or Codex's config
/// *without asking* — the kind of surprise that is fair to be angry about.
/// The first interactive run is the earliest moment a real person is present to
/// answer, so the whole thing costs one keystroke and no documentation hunting.
/// Headless and scripted setups still have `translatus mcp install`.
///
/// Asked at most once, whatever the answer: a declined offer that comes back is
/// nagging, and this is a convenience, not a requirement.
fn offer_mcp_registration() -> Result<()> {
    if !store::mcp_offer_pending() {
        return Ok(());
    }
    let clients = crate::mcp::installed_clients();
    if clients.is_empty() {
        // Nothing to register with. Do NOT mark it done — the user may install
        // an agent next week, and that is exactly when the offer is useful.
        return Ok(());
    }
    store::mark_mcp_offer_done();

    let names = clients.join(i18n::tr("mcp.list.sep"));
    let lines = vec![
        format!("  {}", theme::purple_bold(i18n::tr("mcp.offer.title"))),
        String::new(),
        format!("  {}", i18n::tr1("mcp.offer.found", &names)),
        format!("  {}", i18n::tr("mcp.offer.what")),
        String::new(),
        format!("  {}", theme::gray(i18n::tr("mcp.offer.undo"))),
    ];
    if widgets::confirm(&lines, i18n::tr("mcp.offer.gate"))? {
        let report = crate::mcp::register(true, None)?;
        widgets::notice(i18n::tr("mcp.offer.done"), &report)?;
    }
    Ok(())
}

/// What this run will, and will not, be billed for once the job cache beside
/// its output is taken into account.
#[derive(Default)]
struct RunResume {
    /// Chapter indices a previous run finished, which this one restores free.
    done: Vec<usize>,
    /// A job exists, but under a different model/depth/note profile: its cache
    /// misses entirely, so the book runs again from the start.
    stale: bool,
    /// The job under the DEFAULT output name holds finished chapters that this
    /// run will not touch, because a custom "Save to" moves the job with it.
    orphaned: Option<usize>,
}

/// The signatures this run will look its caches up under: `(translation,
/// annotation)`. `None` on a side means the run does not depend on that cache.
/// `None` overall means the choices do not form a runnable config — treated as
/// "no resume", because promising a discount we cannot verify is worse than
/// quoting the full book.
fn wanted_signatures(c: &flow::Choices) -> Option<(Option<String>, Option<String>)> {
    if c.translate {
        let cfg = crate::translate_config(&to_args(c)).ok()?;
        Some((Some(cfg.cache_signature()), cfg.annotation_signature()))
    } else {
        let cfg = crate::annotate_config(&to_annotate_args(c)).ok()?;
        Some((None, cfg.annotation_signature()))
    }
}

/// Read the job this run will open and decide what it means for the bill.
fn run_resume(c: &flow::Choices) -> RunResume {
    let mut out = RunResume::default();
    let Some((want_cfg, want_anno)) = wanted_signatures(c) else {
        return out;
    };
    let job = resume::job_path(&c.input, &c.to, c.translate, c.output.as_deref());
    if let Some(r) = resume::Resume::probe(&job) {
        if r.done_count() > 0 {
            if r.reuses(want_cfg.as_deref(), want_anno.as_deref()) {
                out.done = r.done;
            } else {
                out.stale = true;
            }
            return out;
        }
    }
    // The job path is derived from the output path, so typing a different
    // "Save to" silently starts a new job. That is the engine's contract and
    // not something a screen can fix — but it can refuse to let it happen
    // quietly.
    if c.output.is_some() {
        let default_job = resume::job_path(&c.input, &c.to, c.translate, None);
        if default_job != job {
            if let Some(r) = resume::Resume::probe(&default_job) {
                if r.done_count() > 0 {
                    out.orphaned = Some(r.done_count());
                }
            }
        }
    }
    out
}

/// The resume block on the confirm screen: what is already paid for, what this
/// run is actually buying, and what would throw the saving away.
fn resume_lines(res: &RunResume, done: usize, left: usize) -> Vec<String> {
    let mut out = Vec::new();
    if done > 0 {
        out.push(format!(
            "  {}",
            theme::green(
                &tr("go.resume")
                    .replacen("{}", &trn("n.chapter", done), 1)
                    .replacen("{}", &trn("n.chapter", left), 1)
            )
        ));
        out.push(format!("  {}", theme::gray(tr("go.resume.keys"))));
    } else if res.stale {
        out.push(format!("  {}", theme::yellow(tr("go.resume.stale"))));
    } else if let Some(n) = res.orphaned {
        out.push(format!(
            "  {}",
            theme::yellow(&tr1("go.resume.newjob", &trn("n.chapter", n)))
        ));
    }
    if !out.is_empty() {
        out.push(String::new());
    }
    out
}

/// Parse the book and describe the run in the reader's terms.
///
/// Deliberately no time estimate here: we have no measured throughput for this
/// book, model, and connection yet, and a fabricated "~38 min" is worse than
/// no number. The progress board shows remaining time once it has evidence to
/// extrapolate from.
///
/// Every number here is for the work this run will actually do. On a resume
/// that means the remaining chapters only: quoting the whole book when eight
/// of its twelve chapters are already paid for is not a conservative estimate,
/// it is the wrong number.
fn estimate_lines(c: &flow::Choices) -> Result<Vec<String>> {
    let (book, _doc) = format::extract(&c.input)?;
    let res = run_resume(c);
    let density = c.annotate.then_some(Density::Medium);
    let est = crate::estimate_numbers_from_tokens(
        book.est_source_tokens_excluding(&res.done) as u64,
        &c.model,
        level_of(&c.level),
        density,
    );
    let chars: usize = book
        .chapters
        .iter()
        .enumerate()
        .filter(|(i, _)| !res.done.contains(i))
        .flat_map(|(_, ch)| ch.segments.iter())
        .map(|s| s.source.chars().count())
        .sum();
    let left_chapters = book.chapters.len().saturating_sub(res.done.len());
    let total_cost = est.cost + est.annotation.map(|(_, _, x)| x).unwrap_or(0.0);

    let name = c
        .input
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut out = vec![
        format!("  {}", theme::purple_bold(&name)),
        String::new(),
        {
            // Annotate-only keeps the original text; an "→ target-language"
            // arrow there would promise a translation that never happens.
            let mut line = if c.translate {
                tr("chars.arrow").to_string()
            } else {
                tr("chars.noarrow").to_string()
            };
            line = line.replacen("{}", &trn("n.chapter", left_chapters), 1);
            line = line.replacen("{}", &group(chars), 1);
            if c.translate {
                line = line.replacen("{}", &c.to, 1);
            }
            line
        },
        format!("  {} · {}", c.provider, c.model),
    ];
    out.push(String::new());
    out.extend(resume_lines(&res, res.done.len(), left_chapters));
    if matches!(c.provider.as_str(), "mock") {
        out.push(format!("  {}", theme::yellow(tr("go.mock"))));
    } else if is_loopback_endpoint(c.base_url.as_deref()) {
        // A subscription is a flat fee, so a dollar figure here is fiction
        // dressed as precision. What the user is actually spending is quota
        // and wall time, and volume is the honest proxy for both.
        let tokens = est.tokens_out + est.annotation.map(|(_, o, _)| o).unwrap_or(0);
        out.push(tr1("go.work", &group(tokens as usize)));
        out.push(format!("  {}", theme::gray(tr("go.floor"))));
    } else if total_cost > 0.0 {
        out.push(tr1("go.est", &format!("{total_cost:.2}")));
        out.push(format!("  {}", theme::gray(tr("go.floor"))));
    } else {
        out.push(format!("  {}", theme::gray(&tr1("go.noprice", &c.model))));
    }
    for ch in book.apparatus_chapters() {
        let name = ch.href.rsplit('/').next().unwrap_or(&ch.href).to_string();
        out.push(format!("  {}", theme::gray(&tr1("go.skipped", &name))));
    }
    out.push(String::new());
    out.push(format!(
        "  {} {}",
        theme::gray(theme::REVIEW),
        theme::gray(tr("go.stopsafe"))
    ));
    out.push(String::new());
    // The flag form of the same run. Anyone who does this twice should
    // graduate to the CLI, and the fastest way to teach it is to show the
    // answer next to the thing they just assembled by hand.
    out.push(format!(
        "  {}",
        theme::gray(&tr1("go.samecmd", &command_line(c)))
    ));
    Ok(out)
}

/// The one-line consequence shown on the confirm row. Quantified on purpose:
/// the user is agreeing to a specific amount of work and money.
fn summarise_gate(c: &flow::Choices) -> Result<String> {
    let (book, _doc) = format::extract(&c.input)?;
    // The row the user presses agrees with the block above it, resume included:
    // it prices the chapters still to run, not the ones already bought.
    let res = run_resume(c);
    let density = c.annotate.then(|| density_of(&c.density));
    let est = crate::estimate_numbers_from_tokens(
        book.est_source_tokens_excluding(&res.done) as u64,
        &c.model,
        level_of(&c.level),
        density,
    );
    // Translation-only cost drops out when translation is switched off.
    let total =
        if c.translate { est.cost } else { 0.0 } + est.annotation.map(|(_, _, x)| x).unwrap_or(0.0);
    // The row the user actually presses has to agree with the block above it.
    // Quoting work there and money here put two different framings of the same
    // run on one screen, and the money one was the one being agreed to.
    let cost = if matches!(c.provider.as_str(), "mock") {
        tr("go.free").to_string()
    } else if is_loopback_endpoint(c.base_url.as_deref()) {
        let tokens = est.tokens_out + est.annotation.map(|(_, o, _)| o).unwrap_or(0);
        tr1("gate.work", &group(tokens as usize))
    } else if total > 0.0 {
        tr1("gate.cost", &format!("{total:.2}"))
    } else {
        tr("go.unpriced").to_string()
    };
    Ok(format!(
        "{}{}{cost}",
        gate_phrase(
            c.translate,
            c.annotate,
            book.chapters.len().saturating_sub(res.done.len()),
            &c.to
        ),
        tr("go.comma")
    ))
}

/// What the confirm row promises, in words.
///
/// Every service being bought has to be named. A gate that says "Translate 3
/// chapters" while quietly also running three annotation passes is quoting for
/// less than it is about to do.
fn gate_phrase(translate: bool, annotate: bool, chapters: usize, to: &str) -> String {
    let n = trn("n.chapter", chapters);
    let fill = |key: &str| tr(key).replacen("{}", &n, 1).replace("{to}", to);
    match (translate, annotate) {
        (true, true) => fill("gate.both"),
        (true, false) => fill("gate.translate"),
        (false, true) => fill("gate.annotate"),
        (false, false) => tr("gate.nothing").to_string(),
    }
}

/// The flag form of the current choices, so the TUI teaches the CLI.
fn command_line(c: &flow::Choices) -> String {
    // Notes-without-translation is a different subcommand, and printing
    // `translate --annotate` for it would teach a command that does something
    // else.
    let mut s = if c.translate {
        format!(
            "translatus translate {:?} --to {} --provider {} --model {}",
            sanitize_plain(&c.input.to_string_lossy()),
            c.to,
            c.provider,
            c.model
        )
    } else {
        format!(
            "translatus annotate {:?} --provider {} --model {}",
            sanitize_plain(&c.input.to_string_lossy()),
            c.provider,
            c.model
        )
    };
    if c.translate {
        if c.level != "sentence" {
            s.push_str(&format!(" --level {}", c.level));
        }
        if c.mode != "replace" {
            s.push_str(&format!(" --mode {}", c.mode));
        }
        if c.annotate {
            s.push_str(" --annotate");
        }
    }
    if let Some(u) = &c.base_url {
        s.push_str(&format!(" --base-url {u}"));
    }
    if c.annotate {
        if let Some(p) = &c.profile {
            s.push_str(&format!(" --profile {p:?}"));
        }
        if let Some(l) = &c.note_level {
            s.push_str(&format!(" --note-level {l}"));
        }
        if let Some(a) = &c.note_anchors {
            s.push_str(&format!(" --note-anchors {a:?}"));
        }
        if let Some(v) = &c.note_voice {
            s.push_str(&format!(" --note-voice {v}"));
        }
        if let Some(p) = &c.note_presets {
            s.push_str(&format!(" --note-presets {p}"));
        }
        if let Some(l) = &c.note_lang {
            s.push_str(&format!(" --note-lang {l}"));
        }
        if c.density != "medium" {
            s.push_str(&format!(" --density {}", c.density));
        }
    }
    s
}

/// The engine's density enum for a stored id.
fn density_of(id: &str) -> Density {
    match id {
        "sparse" => Density::Sparse,
        "rich" => Density::Rich,
        _ => Density::Medium,
    }
}

/// A locally-served endpoint: the subscription sidecar or a local model server.
/// Both serve one request at a time, so fanning out only produces 429s.
fn is_loopback_endpoint(base_url: Option<&str>) -> bool {
    let Some(url) = base_url else { return false };
    et_core::validate_base_url(url, et_core::EndpointTrust::CallerSupplied).is_ok()
}

/// The engine's level for a session choice, defaulting to the cheap path when
/// the string is not one the engine knows.
fn level_of(s: &str) -> et_core::config::Level {
    s.parse().unwrap_or(et_core::config::Level::Sentence)
}

fn to_args(c: &flow::Choices) -> crate::TranslateArgs {
    crate::TranslateArgs {
        input: c.input.clone(),
        to: c.to.clone(),
        level: c.level.clone(),
        provider: c.provider.clone(),
        model: c.model.clone(),
        base_url: c.base_url.clone(),
        prompt: None,
        output: c.output.clone(),
        mode: c.mode.clone(),
        // One at a time for anything served locally. A subscription sidecar
        // accepts exactly one paid completion at a time (and Ollama runs one
        // model), so four workers against it means three 429s per window —
        // which, before they were backed off properly, silently cost units.
        // The guide says to keep subscription backends at 1; the session that
        // recommends the sidecar has to follow its own advice.
        concurrency: if c.provider == "ollama" || is_loopback_endpoint(c.base_url.as_deref()) {
            1
        } else {
            4
        },
        job: None,
        cache_only: false,
        annotate: c.annotate,
        profile: c.profile.clone(),
        note_lang: c.note_lang.clone(),
        note_style: None,
        note_presets: c.note_presets.clone(),
        // The full contract document stays flag/agent-surface only; the TUI
        // collects goals/anchors/voice as its own questions.
        note_profile: None,
        note_level: c.note_level.clone(),
        note_anchors: c.note_anchors.clone(),
        note_voice: c.note_voice.clone(),
        density: Some(c.density.clone()),
    }
}

/// The annotate-only run: same collected intent, routed to the `annotate`
/// command's arguments.
fn to_annotate_args(c: &flow::Choices) -> crate::AnnotateArgs {
    crate::AnnotateArgs {
        input: c.input.clone(),
        profile: c.profile.clone(),
        note_lang: c.note_lang.clone(),
        note_style: None,
        note_presets: c.note_presets.clone(),
        note_profile: None,
        note_level: c.note_level.clone(),
        note_anchors: c.note_anchors.clone(),
        note_voice: c.note_voice.clone(),
        density: Some(c.density.clone()),
        provider: c.provider.clone(),
        model: c.model.clone(),
        base_url: c.base_url.clone(),
        output: c.output.clone(),
        job: None,
        cache_only: false,
    }
}

/// Thousands separators. Six-figure character counts are the norm for a book,
/// and `118543` is materially harder to read at a glance than `118,543`.
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    /// The gate is the last thing between a person and a bill. If a service is
    /// switched on, the sentence has to say so.
    #[test]
    fn the_gate_names_every_service_being_bought() {
        i18n::force(i18n::Lang::En);
        let t = gate_phrase(true, false, 3, "English");
        assert!(t.contains("Translate") && t.contains("3 chapters"), "{t}");
        assert!(!t.contains("notes"), "notes are off: {t}");

        let both = gate_phrase(true, true, 3, "English");
        assert!(both.contains("Translate"), "{both}");
        assert!(
            both.contains("margin notes"),
            "annotation is not mentioned: {both}"
        );

        let notes = gate_phrase(false, true, 3, "English");
        assert!(notes.contains("margin notes"), "{notes}");
        assert!(
            !notes.contains("Translate"),
            "translation is off; promising it would be a lie: {notes}"
        );
        assert!(notes.contains("keeping the original"), "{notes}");

        assert_eq!(gate_phrase(false, false, 3, "English"), "Nothing selected");
    }

    fn choices(translate: bool, annotate: bool) -> flow::Choices {
        choices_for(std::path::PathBuf::from("/tmp/b.epub"), translate, annotate)
    }

    pub(super) fn choices_for(
        input: std::path::PathBuf,
        translate: bool,
        annotate: bool,
    ) -> flow::Choices {
        flow::Choices {
            input,
            translate,
            annotate,
            to: "English".into(),
            level: "sentence".into(),
            mode: "replace".into(),
            provider: "openai".into(),
            model: "gpt-5.4-mini".into(),
            base_url: None,
            profile: Some("a curious reader".into()),
            note_level: Some("beginner".into()),
            note_anchors: Some("software".into()),
            note_voice: Some("companion".into()),
            note_presets: Some("terms,history".into()),
            note_lang: None,
            density: "rich".into(),
            output: None,
        }
    }

    /// The echoed command is a teaching aid, so it has to be the run that is
    /// actually about to happen — including every annotation flag.
    #[test]
    fn the_echoed_command_reproduces_the_run() {
        i18n::force(i18n::Lang::En);
        let c = choices(true, true);
        let s = command_line(&c);
        assert!(s.starts_with("translatus translate"), "{s}");
        assert!(s.contains("--annotate"), "{s}");
        assert!(s.contains("--profile"), "annotation profile missing: {s}");
        assert!(s.contains("--note-presets terms,history"), "{s}");
        assert!(s.contains("--density rich"), "{s}");

        // Notes without translation is a different subcommand entirely.
        let a = command_line(&choices(false, true));
        assert!(a.starts_with("translatus annotate"), "{a}");
        assert!(!a.contains("--annotate"), "annotate has no such flag: {a}");
        assert!(
            !a.contains("--to "),
            "annotate does not take a target language: {a}"
        );
        assert!(a.contains("--profile"), "{a}");

        // Translation alone must not carry annotation flags.
        let t = command_line(&choices(true, false));
        assert!(!t.contains("--profile"), "{t}");
        assert!(!t.contains("--density"), "{t}");
    }

    #[test]
    fn density_ids_map_to_the_engine_enum() {
        assert!(matches!(density_of("sparse"), Density::Sparse));
        assert!(matches!(density_of("medium"), Density::Medium));
        assert!(matches!(density_of("rich"), Density::Rich));
        // An unknown id from a newer settings file lands on the default.
        assert!(matches!(density_of("whatever"), Density::Medium));
    }
}

/// The resume screens: what the confirm gate is allowed to promise about work
/// a previous run already paid for.
#[cfg(test)]
mod resume_tests {
    use super::gate_tests::choices_for;
    use super::*;
    use et_core::job::JobStore;
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("translatus-gate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Write a job file that looks like an interrupted run of `c`.
    fn seed_job(
        job: &std::path::Path,
        done: usize,
        config_sig: Option<&str>,
        anno_sig: Option<&str>,
    ) {
        let store = JobStore::open(job).unwrap();
        store.set_meta("total_chapters", "12").unwrap();
        if let Some(s) = config_sig {
            store.set_meta("config_sig", s).unwrap();
        }
        if let Some(s) = anno_sig {
            store.set_meta("anno_sig", s).unwrap();
        }
        for i in 0..done {
            store
                .set_chapter_status(i, &format!("c{i}.xhtml"), "done")
                .unwrap();
        }
    }

    /// The whole point: a run that will restore eight chapters must not be
    /// quoted, or described, as if it were going to translate them again.
    #[test]
    fn a_matching_job_is_counted_as_already_paid_for() {
        let d = tmpdir("match");
        let c = choices_for(d.join("b.epub"), true, false);
        let (want_cfg, want_anno) = wanted_signatures(&c).expect("a runnable config");
        assert!(want_anno.is_none(), "notes are off in this run");
        seed_job(
            &resume::job_path(&c.input, &c.to, true, None),
            8,
            want_cfg.as_deref(),
            None,
        );

        let res = run_resume(&c);
        assert_eq!(res.done.len(), 8);
        assert!(!res.stale);
        assert!(res.orphaned.is_none());
    }

    /// A job written under another model is not a resume: the cache misses on
    /// every segment and the book runs again in full. Offering it as progress
    /// would under-quote the run by whatever fraction is "done".
    #[test]
    fn a_job_from_other_settings_is_flagged_stale_not_reused() {
        let d = tmpdir("stale");
        let c = choices_for(d.join("b.epub"), true, false);
        seed_job(
            &resume::job_path(&c.input, &c.to, true, None),
            8,
            Some("a-signature-from-another-model"),
            None,
        );

        let res = run_resume(&c);
        assert!(res.done.is_empty(), "nothing may be reused");
        assert!(
            res.stale,
            "the screen has to say why the whole book runs again"
        );
    }

    /// Notes are a second cache. A chapter whose translation is cached but
    /// whose notes are about to be rewritten is not "already done".
    #[test]
    fn changed_note_settings_void_the_resume_too() {
        let d = tmpdir("anno");
        let c = choices_for(d.join("b.epub"), true, true);
        let (want_cfg, want_anno) = wanted_signatures(&c).expect("a runnable config");
        assert!(want_anno.is_some(), "notes are on in this run");
        seed_job(
            &resume::job_path(&c.input, &c.to, true, None),
            8,
            want_cfg.as_deref(),
            Some("notes-written-for-a-different-reader"),
        );

        let res = run_resume(&c);
        assert!(res.done.is_empty());
        assert!(res.stale);
    }

    /// The trap: the job cache is derived from the output path, so typing a
    /// different "Save to" silently abandons the finished chapters. The engine
    /// cannot fix that without moving everyone's existing caches, so the screen
    /// has to say it.
    #[test]
    fn a_custom_save_to_warns_that_finished_work_is_left_behind() {
        let d = tmpdir("moved");
        let mut c = choices_for(d.join("b.epub"), true, false);
        let (want_cfg, _) = wanted_signatures(&c).expect("a runnable config");
        seed_job(
            &resume::job_path(&c.input, &c.to, true, None),
            8,
            want_cfg.as_deref(),
            None,
        );

        c.output = Some(d.join("somewhere-else.epub"));
        let res = run_resume(&c);
        assert!(res.done.is_empty(), "the new job cache is empty");
        assert_eq!(res.orphaned, Some(8), "the abandoned work has to be named");
    }

    /// An annotate-only run does not translate, so the translation signature is
    /// irrelevant to it; only the note cache decides what it can resume.
    #[test]
    fn annotate_only_resumes_on_the_note_cache_alone() {
        let c = choices_for(std::path::PathBuf::from("/tmp/b.epub"), false, true);
        let (want_cfg, want_anno) = wanted_signatures(&c).expect("a runnable config");
        assert!(
            want_cfg.is_none(),
            "nothing is translated, so nothing is keyed on it"
        );
        assert!(want_anno.is_some());
    }

    /// The block on screen has to carry both numbers — what is already bought
    /// and what is being bought now — and the warning that throws it away.
    /// Asserted on the digits so it holds in all five languages.
    #[test]
    fn the_resume_block_names_both_numbers() {
        let res = RunResume {
            done: (0..8).collect(),
            ..Default::default()
        };
        let lines = resume_lines(&res, 8, 4);
        assert_eq!(lines.len(), 3, "two lines and a spacer: {lines:?}");
        assert!(
            lines[0].contains('8') && lines[0].contains('4'),
            "{lines:?}"
        );
        assert!(
            !lines[1].trim().is_empty(),
            "the voiding warning is missing"
        );

        let stale = RunResume {
            stale: true,
            ..Default::default()
        };
        assert_eq!(resume_lines(&stale, 0, 12).len(), 2);

        let moved = RunResume {
            orphaned: Some(8),
            ..Default::default()
        };
        let moved_lines = resume_lines(&moved, 0, 12);
        assert!(moved_lines[0].contains('8'), "{moved_lines:?}");

        // Nothing to resume: no block at all, not an empty one.
        assert!(resume_lines(&RunResume::default(), 0, 12).is_empty());
    }
}
