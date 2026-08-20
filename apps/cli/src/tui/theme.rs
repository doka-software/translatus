//! Visual language: colours, icons, and the brand banner.
//!
//! Deliberately small and semantically fixed — nine colours, each with one
//! job, so a reader learns the palette once. Grey carries every piece of
//! chrome (rules, hints, key legends), which leaves content as the only
//! high-contrast element on screen. There is no background highlight: the
//! selected row is marked by a leading arrow plus a cyan foreground, which
//! stays legible on both light and dark terminal themes.
//!
//! Adapted from the mole CLI teardown (`wiki/mole-cli-ux-teardown.md` §1).
//! Two deliberate departures, both recorded there in §8.4: we use `✗` rather
//! than mole's softened `☻` for errors (our failures are API/format problems
//! where clarity beats reassurance), and the banner puts its byline below the
//! wordmark rather than beside it, because "translatus" is 47 columns wide and
//! leaves no room to the right on an 80-column terminal.

use std::io::IsTerminal;
use std::sync::OnceLock;

/// Whether to emit ANSI at all. Cached — this is consulted on every styled
/// span, and the environment cannot change mid-run.
///
/// Honours `NO_COLOR` (https://no-color.org) and a `dumb`/empty `TERM`, and
/// requires stdout to be a terminal. `CLICOLOR_FORCE` overrides all of it so
/// the output can still be captured for documentation and screenshots.
fn ansi() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        if std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| v != "0") {
            return true;
        }
        if std::env::var_os("NO_COLOR").is_some() {
            return false;
        }
        match std::env::var("TERM") {
            Ok(t) if t.is_empty() || t == "dumb" || t == "unknown" => return false,
            Err(_) => return false,
            _ => {}
        }
        std::io::stdout().is_terminal()
    })
}

macro_rules! style {
    ($($(#[$doc:meta])* $name:ident => $code:literal;)*) => {
        $(
            $(#[$doc])*
            pub fn $name(s: &str) -> String {
                if ansi() { format!("\x1b[{}m{s}\x1b[0m", $code) } else { s.to_string() }
            }
        )*
    };
}

style! {
    /// Success, and the affirmative key in a confirm prompt.
    green => "0;32";
    /// Section headings and informational leads.
    blue => "1;34";
    /// The one and only selection colour. Never used for anything else, so
    /// "cyan" reads unambiguously as "this is where you are".
    cyan => "0;36";
    /// Warnings and dry-run banners.
    yellow => "0;33";
    /// Errors.
    red => "0;31";
    /// The arrow prefix on confirm prompts and section headers.
    purple => "0;35";
    /// Screen titles.
    purple_bold => "1;35";
    /// All chrome: key legends, paths, rules, pending rows, hints.
    gray => "0;90";
}

/// Cursor / current item. Two columns wide including its trailing space, so an
/// unselected row padded with two spaces sits at the same x-offset — moving
/// the selection never shifts text horizontally.
pub const ARROW: &str = "➤";
/// Completed.
pub const OK: &str = "✓";
/// Failed. (mole uses `☻`; see the module note.)
pub const FAIL: &str = "✗";
/// "Look at this yourself." Marks anything the tool will not decide for you —
/// the single most important glyph in the set for a long-running job, because
/// translation quality is not machine-checkable.
pub const REVIEW: &str = "☞";

/// Frames for the in-place spinner. ASCII on purpose: braille and block
/// spinners render at inconsistent widths across terminals, which makes the
/// line jitter.
pub const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

/// FIGlet `standard`, 47×5. Rendered plain when the terminal is too narrow to
/// hold it, rather than wrapping into noise.
const WORDMARK: &str = r#" _                       _       _
| |_ _ __ __ _ _ __  ___| | __ _| |_ _   _ ___
| __| '__/ _` | '_ \/ __| |/ _` | __| | | / __|
| |_| | | (_| | | | \__ \ | (_| | |_| |_| \__ \
 \__|_|  \__,_|_| |_|___/_|\__,_|\__|\__,_|___/"#;

const WORDMARK_COLS: usize = 47;

/// The banner, as a list of ready-to-print lines (no trailing newline).
pub fn banner(cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    if cols >= WORDMARK_COLS {
        for line in WORDMARK.lines() {
            out.push(blue(line));
        }
    } else {
        out.push(blue("translatus"));
    }
    out.push(String::new());
    out.push(format!(
        "  {}  {}  {}",
        green("From any book, to your book."),
        gray("·"),
        gray(&format!("v{}", env!("CARGO_PKG_VERSION")))
    ));
    out
}
