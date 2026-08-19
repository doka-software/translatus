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
        let report = crate::mcp::register(true)?;
        widgets::notice(i18n::tr("mcp.offer.done"), &report)?;
    }
    Ok(())
}

/// Parse the book and describe the run in the reader's terms.
///
/// Deliberately no time estimate here: we have no measured throughput for this
/// book, model, and connection yet, and a fabricated "~38 min" is worse than
/// no number. The progress board shows remaining time once it has evidence to
/// extrapolate from.
fn estimate_lines(c: &flow::Choices) -> Result<Vec<String>> {
    let (book, _doc) = format::extract(&c.input)?;
    let density = c.annotate.then_some(Density::Medium);
    let est = crate::estimate_numbers(&book, &c.model, density);
    let chars: usize = book
        .chapters
        .iter()
        .flat_map(|ch| ch.segments.iter())
        .map(|s| s.source.chars().count())
        .sum();
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
            line = line.replacen("{}", &trn("n.chapter", book.chapters.len()), 1);
            line = line.replacen("{}", &group(chars), 1);
            if c.translate {
                line = line.replacen("{}", &c.to, 1);
            }
            line
        },
        format!("  {} · {}", c.provider, c.model),
    ];
    out.push(String::new());
    if matches!(c.provider.as_str(), "mock") {
        out.push(format!(
            "  {}",
            theme::yellow("mock provider — nothing is sent anywhere and nothing is billed")
        ));
    } else if total_cost > 0.0 {
        out.push(tr1("go.est", &format!("{total_cost:.2}")));
        out.push(format!("  {}", theme::gray(tr("go.est.note"))));
    } else {
        out.push(format!("  {}", theme::gray(&tr1("go.noprice", &c.model))));
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
    let density = c.annotate.then(|| density_of(&c.density));
    let est = crate::estimate_numbers(&book, &c.model, density);
    // Translation-only cost drops out when translation is switched off.
    let total =
        if c.translate { est.cost } else { 0.0 } + est.annotation.map(|(_, _, x)| x).unwrap_or(0.0);
    let cost = if matches!(c.provider.as_str(), "mock") {
        tr("go.free").to_string()
    } else if total > 0.0 {
        format!("~${total:.2}")
    } else {
        tr("go.unpriced").to_string()
    };
    Ok(format!(
        "{}{}{cost}",
        gate_phrase(c.translate, c.annotate, book.chapters.len(), &c.to),
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
        flow::Choices {
            input: std::path::PathBuf::from("/tmp/b.epub"),
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
