//! Terminal state and the drawing primitives everything else is built from.
//!
//! Two rules govern this module, both taken from the mole teardown
//! (`wiki/mole-cli-ux-teardown.md` §4.5, §7.4–7.6):
//!
//! 1. **Never clear the whole screen to redraw.** Home the cursor, overwrite
//!    each line with `\r\x1b[2K` in front of it, then clear from the cursor
//!    down. A full `\x1b[2J` flashes the terminal on every keypress; this does
//!    not, because only the cells that actually changed get repainted.
//! 2. **Whatever we change, we change back.** Raw mode, the alternate screen,
//!    and cursor visibility are all restored by `Terminal`'s `Drop`, so a
//!    panic or a `?` leaves the user's shell exactly as we found it.
//!
//! Unlike mole — which is bash and has to approximate CJK width by comparing
//! byte and character counts — we measure it exactly with `unicode-width`.
//! Same goal, better tool.

use anyhow::Result;
use crossterm::{cursor, execute, terminal};
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use unicode_width::UnicodeWidthStr;

/// Whether the alternate screen and raw mode are currently ours.
///
/// Process-global rather than per-guard because two places need to put the
/// terminal back: the RAII guard on the way out, and the run itself, which has
/// to drop back to the real screen *before* printing its summary so the
/// summary survives in the user's scrollback. Both call [`restore`]; whichever
/// gets there first does the work and the other becomes a no-op.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Owns the terminal's mutated state and restores it on drop.
///
/// Everything is written to **stderr**, leaving stdout clean for anything the
/// user pipes or captures.
pub struct Terminal {
    _private: (),
}

impl Terminal {
    /// Enter raw mode + the alternate screen and hide the cursor.
    ///
    /// The alternate screen is the right call here (and the one place we
    /// diverge from mole's main menu, which deliberately paints over the
    /// scrollback): a translation run emits a lot of progress, and the user
    /// should get their scrollback back untouched when it ends.
    pub fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(
            std::io::stderr(),
            terminal::EnterAlternateScreen,
            cursor::Hide
        )?;
        ACTIVE.store(true, Ordering::SeqCst);
        Ok(Self { _private: () })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}

/// Put the terminal back the way we found it. Safe to call any number of
/// times, from anywhere, including a panic unwind.
pub fn restore() {
    if !ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = execute!(
        std::io::stderr(),
        cursor::Show,
        terminal::LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
}

/// Terminal size, with a conventional fallback when there is no tty (pipes,
/// CI). 80×24 is the floor every layout in this crate is designed against.
///
/// A zero on either axis is treated as "unknown", not as a real measurement:
/// some pseudo-terminals (notably `script`, and a few CI runners) report 0×0,
/// and taking that literally collapses every layout to its narrowest fallback.
pub fn size() -> (usize, usize) {
    let (c, r) = terminal::size().unwrap_or((0, 0));
    (
        if c == 0 { 80 } else { c as usize },
        if r == 0 { 24 } else { r as usize },
    )
}

/// Is this an interactive session? Both ends must be a terminal: stdout so we
/// have somewhere to draw, stdin so there is someone to press a key.
pub fn is_interactive() -> bool {
    std::io::stdout().is_terminal() && std::io::stdin().is_terminal()
}

/// Accumulates a frame, then writes it in one syscall.
///
/// Building the whole frame in memory and flushing once is what keeps the
/// redraw atomic — a partially-written frame is visible tearing.
pub struct Frame {
    buf: String,
    cols: usize,
}

impl Frame {
    /// Start a frame at the top-left. The screen is *not* cleared; each line
    /// clears itself as it is written, and `finish` clears whatever is left
    /// below.
    pub fn new() -> Self {
        Self::with_cols(size().0)
    }

    /// A frame with an explicit width. Exists so the clamping can be tested
    /// against `line` itself rather than only against the helper it calls:
    /// testing the helper alone left the wiring uncovered, and the original
    /// bug was precisely a missing call, not a wrong calculation.
    pub fn with_cols(cols: usize) -> Self {
        Self {
            buf: String::from("\x1b[H"),
            cols,
        }
    }

    /// The bytes this frame would write. Test-only.
    #[cfg(test)]
    pub fn rendered(&self) -> &str {
        &self.buf
    }

    /// Write one line, clearing the row it lands on first.
    ///
    /// Clamped to the terminal width, because the whole clear-as-you-go scheme
    /// assumes one logical line occupies one physical row. A line wider than
    /// the terminal auto-wraps onto the next row, which this frame has not
    /// cleared — so the tail of the *previous* frame survives next to the
    /// wrapped text, and every row below is off by one. At 60 columns that
    /// showed up as the confirm screen's "…billed" running into the settings
    /// screen's "Layout   translation only".
    pub fn line(&mut self, s: &str) -> &mut Self {
        self.buf.push_str("\r\x1b[2K");
        self.buf.push_str(&clamp_styled(s, self.cols));
        self.buf.push_str("\r\n");
        self
    }

    /// A blank (but cleared) row.
    pub fn blank(&mut self) -> &mut Self {
        self.line("")
    }

    /// Clear from the cursor to the bottom and flush. Without this, a frame
    /// that is shorter than its predecessor leaves the old tail on screen.
    pub fn finish(&mut self) -> Result<()> {
        self.buf.push_str("\x1b[J");
        let mut err = std::io::stderr();
        err.write_all(self.buf.as_bytes())?;
        err.flush()?;
        Ok(())
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

/// Display width in terminal cells — CJK counts as 2, combining marks as 0.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate to `max` display columns, appending `…` when anything was cut.
///
/// Width-aware rather than byte- or char-aware: cutting a 15-character
/// Chinese title at "15 chars" overflows a 20-column field by 10 cells.
pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let budget = max - 1; // reserve one cell for the ellipsis
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = UnicodeWidthStr::width(ch.to_string().as_str());
        if w + cw > budget {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Pad to `target` display columns. Never truncates — callers that need a hard
/// ceiling call `truncate` first.
///
/// `format!("{:<width$}")` pads by `char` count, which misaligns any column
/// containing CJK; this pads by display width instead.
pub fn pad(s: &str, target: usize) -> String {
    let w = width(s);
    if w >= target {
        return s.to_string();
    }
    format!("{s}{}", " ".repeat(target - w))
}

/// Remove terminal control and bidi-format characters from untrusted plain
/// text before trusted theme code adds ANSI styling around it.
pub fn sanitize_plain(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    *c as u32,
                    0x202A..=0x202E | 0x2066..=0x2069 | 0x200E..=0x200F | 0x061C
                )
        })
        .collect()
}

/// A horizontal rule, capped at 70 columns so it does not stretch absurdly
/// across an ultrawide terminal.
pub fn rule(cols: usize) -> String {
    "─".repeat(cols.min(70))
}

/// Clamp a styled line to `max` display columns.
///
/// `truncate` cannot be used here: these strings carry ANSI colour escapes,
/// which occupy no cells but plenty of bytes. Counting them would truncate far
/// too early, and cutting inside one would emit a mangled escape to the
/// terminal. So escapes are passed through untouched and only printable
/// characters are measured.
pub fn clamp_styled(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    // Fast path: nothing to do for the overwhelming majority of lines.
    if visible_width(s) <= max {
        return s.to_string();
    }

    let mut out = String::with_capacity(s.len());
    let mut w = 0usize;
    let mut chars = s.chars().peekable();
    let budget = max - 1; // reserve a cell for the ellipsis
    let mut styled = false;

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI: ESC '[' params… final. Copy verbatim through the final byte.
            out.push(c);
            styled = true;
            // The introducer '[' is itself inside the final-byte range, so it
            // has to be consumed before the scan starts or every sequence
            // "ends" immediately after the ESC.
            if chars.peek() == Some(&'[') {
                out.push(chars.next().unwrap());
            }
            for e in chars.by_ref() {
                out.push(e);
                if ('\x40'..='\x7e').contains(&e) {
                    break;
                }
            }
            continue;
        }
        let cw = UnicodeWidthStr::width(c.to_string().as_str());
        if w + cw > budget {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    if styled {
        // Never let a colour leak past a cut into the rest of the screen.
        out.push_str("\x1b[0m");
    }
    out
}

/// Display width ignoring ANSI escape sequences.
pub fn visible_width(s: &str) -> usize {
    let mut w = 0usize;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // See clamp_styled: '[' is in the final-byte range, so skip it
            // explicitly before scanning for the real terminator.
            if chars.peek() == Some(&'[') {
                chars.next();
            }
            for e in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&e) {
                    break;
                }
            }
            continue;
        }
        w += UnicodeWidthStr::width(c.to_string().as_str());
    }
    w
}

#[cfg(test)]
mod clamp_tests {
    use super::{clamp_styled, sanitize_plain, visible_width};

    const PURPLE: &str = "\x1b[35m";
    const RESET: &str = "\x1b[0m";

    #[test]
    fn escapes_cost_no_columns() {
        assert_eq!(visible_width("abc"), 3);
        assert_eq!(visible_width(&format!("{PURPLE}abc{RESET}")), 3);
        // CJK is two cells wide.
        assert_eq!(visible_width("繁體中文"), 8);
        assert_eq!(visible_width(&format!("{PURPLE}繁體{RESET}")), 4);
    }

    #[test]
    fn a_line_never_exceeds_the_terminal_width() {
        // This is the property the clear-as-you-go frame depends on: one
        // logical line, one physical row. Anything wider wraps onto a row the
        // frame did not clear, and the previous screen shows through.
        for max in [1usize, 2, 5, 10, 40, 60, 80] {
            for raw in [
                "a".repeat(200),
                "繁體中文".repeat(50),
                format!("{PURPLE}{}{RESET}", "x".repeat(200)),
                format!("  {PURPLE}➤{RESET} {} more", "字".repeat(80)),
            ] {
                let out = clamp_styled(&raw, max);
                assert!(
                    visible_width(&out) <= max,
                    "max={max} produced width {} from {:?}",
                    visible_width(&out),
                    &raw[..raw.len().min(40)]
                );
            }
        }
    }

    #[test]
    fn short_lines_are_returned_untouched() {
        let styled = format!("  {PURPLE}➤{RESET} Translate");
        assert_eq!(clamp_styled(&styled, 80), styled);
        assert_eq!(clamp_styled("plain", 80), "plain");
    }

    #[test]
    fn a_cut_line_is_marked_and_closes_its_colour() {
        let out = clamp_styled(&format!("{PURPLE}{}", "x".repeat(100)), 20);
        assert!(out.contains('…'), "truncation must be visible: {out:?}");
        assert!(out.ends_with(RESET), "colour must not leak: {out:?}");
        // The escape itself survived intact rather than being cut mid-sequence.
        assert!(out.starts_with(PURPLE), "escape was mangled: {out:?}");
    }

    #[test]
    fn a_cut_never_lands_inside_an_escape_sequence() {
        // Many short colour runs: a byte- or char-based cut would land inside
        // one of these and emit garbage to the terminal.
        let raw: String = (0..50).map(|_| format!("{PURPLE}ab{RESET}")).collect();
        for max in 1..40 {
            let out = clamp_styled(&raw, max);
            // Every ESC in the output is followed by a complete CSI sequence.
            let bytes: Vec<char> = out.chars().collect();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == '\x1b' {
                    let mut j = i + 1;
                    while j < bytes.len() && !('\x40'..='\x7e').contains(&bytes[j]) {
                        j += 1;
                    }
                    assert!(j < bytes.len(), "dangling escape at max={max}: {out:?}");
                    i = j + 1;
                } else {
                    i += 1;
                }
            }
        }
    }

    #[test]
    fn untrusted_plain_text_cannot_inject_terminal_controls() {
        let hostile = "book\x1b]52;c;Y2FuYXJ5\x07.epub\u{202e}txt";
        let safe = sanitize_plain(hostile);
        assert!(!safe.contains('\x1b'));
        assert!(!safe.contains('\x07'));
        assert!(!safe.contains('\u{202e}'));
        assert!(safe.contains("book"));
    }
}

#[cfg(test)]
mod frame_tests {
    use super::{visible_width, Frame};

    /// The regression that matters is at the call site. `clamp_styled` can be
    /// perfect and the screen still corrupt if `line` forgets to call it —
    /// which is exactly what the original bug was.
    #[test]
    fn no_row_a_frame_emits_can_exceed_its_width() {
        const COLS: usize = 60;
        let mut f = Frame::with_cols(COLS);
        f.line("short");
        f.line(&"a".repeat(300));
        f.line(&"繁體中文".repeat(40));
        f.line(&format!("\x1b[35m{}\x1b[0m", "x".repeat(300)));
        f.line("  ☞ Stopping is safe. Progress is checkpointed and resuming is never re-billed.");
        f.blank();

        for row in f.rendered().split("\r\n") {
            // Strip the leading cursor/clear control sequences the frame adds.
            let body = row.trim_start_matches('\u{1b}').trim_start_matches("[H");
            let body = body.strip_prefix('\r').unwrap_or(body);
            let body = body.strip_prefix("\u{1b}[2K").unwrap_or(body);
            assert!(
                visible_width(body) <= COLS,
                "a row would wrap onto an uncleared line: width {} > {COLS} in {body:?}",
                visible_width(body)
            );
        }
    }
}
