//! The live progress board for a translation run.
//!
//! This is the screen the user actually sits with, sometimes for an hour, so
//! it is built around one idea from the mole teardown (§8.5): **a spinner is
//! for "I don't know how long"; a long job needs "how much is left".**
//!
//! So the spinner is demoted from a global status to a property of the single
//! chapter currently in flight. Everything above it is finished and frozen,
//! everything below is pending and grey, and the only moving pixels on screen
//! are one glyph on one row plus a progress bar that advances a few times a
//! minute. A 20fps spinner next to a 40-minute job reads as anxiety; a
//! spinner on the row that is genuinely working reads as progress.
//!
//! Remaining time is extrapolated from the chapters that have actually
//! finished — measured throughput on *this* book with *this* model, which
//! beats any up-front guess.

use super::i18n::{tr, tr1, trn};
use super::term::{self, Frame};
use super::theme::{self, FAIL, OK, SPINNER};
use anyhow::Result;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    Running,
    Done,
    Failed,
}

struct Row {
    title: String,
    chars: usize,
    state: State,
    took: Option<Duration>,
}

pub struct Board {
    book: String,
    rows: Vec<Row>,
    /// What a non-chapter phase is doing right now. Expert level reads the
    /// whole book before the first chapter lands; with nothing on screen but a
    /// 0% bar, that reads as a hang.
    prep: Option<String>,
    started: Instant,
    frame: usize,
    /// Falls back to plain appended lines when there is no terminal to
    /// address — pipes, CI logs, `tee` to a file.
    interactive: bool,
    /// Throttles full repaints. The engine can emit events far faster than a
    /// human can read them.
    last_paint: Instant,
}

impl Board {
    pub fn new(book: &str, chapters: Vec<(String, usize)>) -> Self {
        Self {
            book: term::sanitize_plain(book),
            prep: None,
            rows: chapters
                .into_iter()
                .map(|(title, chars)| Row {
                    title: term::sanitize_plain(&title),
                    chars,
                    state: State::Pending,
                    took: None,
                })
                .collect(),
            started: Instant::now(),
            frame: 0,
            interactive: term::is_interactive(),
            last_paint: Instant::now() - Duration::from_secs(1),
        }
    }

    /// Mark a chapter in flight. Any earlier chapter still marked running is
    /// closed out, so a skipped event cannot strand a spinner on screen.
    pub fn start(&mut self, idx: usize) -> Result<()> {
        for r in self.rows.iter_mut() {
            if r.state == State::Running {
                r.state = State::Done;
            }
        }
        if let Some(r) = self.rows.get_mut(idx) {
            r.state = State::Running;
        }
        self.paint(true)
    }

    /// Name the phase running between chapters (whole-book pre-scan, note
    /// planning). Cleared by the first chapter that finishes.
    pub fn prep(&mut self, label: &str) -> Result<()> {
        let label = term::sanitize_plain(label);
        if self.prep.as_deref() == Some(label.as_str()) {
            return self.tick();
        }
        self.prep = Some(label);
        self.paint(true)
    }

    pub fn finish(&mut self, idx: usize, took: Duration, ok: bool) -> Result<()> {
        self.prep = None;
        if let Some(r) = self.rows.get_mut(idx) {
            r.state = if ok { State::Done } else { State::Failed };
            r.took = Some(took);
        }
        if !self.interactive {
            let r = &self.rows[idx];
            eprintln!(
                "[{}/{}] {} — {} chars, {}",
                idx + 1,
                self.rows.len(),
                r.title,
                r.chars,
                fmt_dur(took)
            );
            return Ok(());
        }
        self.paint(true)
    }

    /// Advance the spinner. Cheap enough to call on a timer; it rewrites one
    /// row rather than the frame.
    pub fn tick(&mut self) -> Result<()> {
        if !self.interactive {
            return Ok(());
        }
        self.frame = self.frame.wrapping_add(1);
        self.paint(false)
    }

    fn done_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| matches!(r.state, State::Done | State::Failed))
            .count()
    }

    /// Extrapolate from measured throughput, in characters rather than chapters —
    /// chapters vary wildly in length, so "12 of 42 chapters" is a poor
    /// predictor while "38% of the text" is a good one.
    fn remaining(&self) -> Option<Duration> {
        let mut done_chars = 0usize;
        let mut done_secs = 0f64;
        for r in self.rows.iter() {
            if let Some(t) = r.took {
                done_chars += r.chars;
                done_secs += t.as_secs_f64();
            }
        }
        let left_chars: usize = self
            .rows
            .iter()
            .filter(|r| r.took.is_none())
            .map(|r| r.chars)
            .sum();
        let total_chars = done_chars + left_chars;
        if done_chars == 0 || total_chars == 0 || done_secs <= 0.0 {
            return None;
        }
        // Sum the chapters' own durations rather than wall-clock elapsed.
        // Expert level scans the whole book before the first chapter lands, and
        // charging that one-off to the handful of front-matter characters that
        // finish first read "~38h56m left" on a run that took under an hour.
        // Stay silent until 5% of the text is measured, for the same reason a
        // fabricated estimate is worse than none.
        if done_chars.saturating_mul(20) < total_chars {
            return None;
        }
        let per_char = done_secs / done_chars as f64;
        Some(Duration::from_secs_f64(per_char * left_chars as f64))
    }

    fn paint(&mut self, force: bool) -> Result<()> {
        if !self.interactive {
            return Ok(());
        }
        if !force && self.last_paint.elapsed() < Duration::from_millis(80) {
            return Ok(());
        }
        self.last_paint = Instant::now();

        let (cols, term_rows) = term::size();
        // header(3) + blank + bar block(3) + legend(2)
        let page = term_rows.saturating_sub(9).clamp(3, 40);
        let running = self
            .rows
            .iter()
            .position(|r| r.state == State::Running)
            .unwrap_or(self.done_count());
        // Keep the active chapter two rows from the bottom of the viewport, so
        // the reader always sees a little of what is coming next.
        let top = running.saturating_sub(page.saturating_sub(3));

        let title_col = self
            .rows
            .iter()
            .map(|r| term::width(&r.title))
            .max()
            .unwrap_or(20)
            .min(cols.saturating_sub(30))
            .max(12);

        let mut f = Frame::new();
        f.blank();
        f.line(&format!(
            "  {} {}",
            theme::purple(theme::ARROW),
            theme::purple_bold(&term::truncate(&self.book, cols.saturating_sub(6)))
        ));
        f.blank();

        for r in self.rows.iter().skip(top).take(page) {
            let mark = match r.state {
                State::Done => theme::green(OK),
                State::Failed => theme::red(FAIL),
                State::Running => theme::blue(&SPINNER[self.frame % SPINNER.len()].to_string()),
                State::Pending => " ".to_string(),
            };
            let title = term::pad(&term::truncate(&r.title, title_col), title_col);
            let meta = match (r.state, r.took) {
                (State::Pending, _) => format!("{:>7}", fmt_count(r.chars)),
                (_, Some(d)) => format!("{:>7}  {:>5}", fmt_count(r.chars), fmt_dur(d)),
                _ => format!("{:>7}", fmt_count(r.chars)),
            };
            // Pending rows recede; finished and active rows stay legible.
            let body = if r.state == State::Pending {
                theme::gray(&format!("{title}  {meta}"))
            } else {
                format!("{title}  {}", theme::gray(&meta))
            };
            f.line(&format!("  {mark} {body}"));
        }

        f.blank();
        let done = self.done_count();
        let total = self.rows.len();
        let pct = (done * 100).checked_div(total).unwrap_or(0);
        let bar_w = cols.saturating_sub(34).clamp(10, 40);
        let filled = bar_w * done / total.max(1);
        let bar = format!(
            "{}{}",
            theme::blue(&"█".repeat(filled)),
            theme::gray(&"░".repeat(bar_w - filled))
        );
        let eta = match self.remaining() {
            Some(d) if done < total => format!("  ·  {}", tr1("run.left", &fmt_dur(d))),
            _ => String::new(),
        };
        f.line(&format!(
            "  {bar}  {}",
            theme::gray(&format!(
                "{done}/{total} {}  ·  {pct}%{eta}",
                tr("run.chapters")
            ))
        ));
        if let Some(prep) = &self.prep {
            f.line(&format!("  {}", theme::gray(prep)));
        }
        f.blank();
        f.line(&theme::gray(tr("run.stop")));
        f.finish()
    }

    /// The closing summary. Rendered after the alternate screen is gone, so it
    /// stays in the user's scrollback.
    pub fn summary(&self, output: &std::path::Path, cost: Option<f64>) -> Vec<String> {
        let (cols, _) = term::size();
        let failed = self
            .rows
            .iter()
            .filter(|r| r.state == State::Failed)
            .count();
        let chars: usize = self.rows.iter().map(|r| r.chars).sum();
        let mut out = vec![
            String::new(),
            theme::gray(&term::rule(cols)),
            theme::blue(if failed == 0 {
                tr("run.done.title")
            } else {
                tr("run.gaps.title")
            }),
        ];
        let mut facts = tr("run.facts")
            .replacen("{}", &trn("n.chapter", self.rows.len()), 1)
            .replacen("{}", &fmt_count(chars), 1)
            .replacen("{}", &fmt_dur(self.started.elapsed()), 1);
        if let Some(c) = cost {
            if c > 0.0 {
                facts.push_str(&format!(", ~${c:.2}"));
            }
        }
        out.push(facts);
        out.push(tr1(
            "run.saved",
            &term::sanitize_plain(&output.to_string_lossy()),
        ));
        if failed > 0 {
            out.push(theme::yellow(&format!(
                "{} {}",
                theme::REVIEW,
                tr1("run.failed", &trn("n.chapter", failed))
            )));
        }
        out.push(theme::gray(&term::rule(cols)));
        out
    }
}

/// Counts, abbreviated once they stop being scannable at full precision.
fn fmt_count(n: usize) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// Durations at the precision a human cares about: seconds under a minute,
/// minutes under an hour, hours-and-minutes above.
pub fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Front matter finishes first and is a few characters long. Charging the
    /// whole wall clock — which at expert level includes a multi-minute
    /// whole-book pre-scan — to those characters produced "~38h56m left" on a
    /// run that finished inside the hour. Two rules keep it honest: rate comes
    /// from the chapters' own durations, and nothing is shown until enough of
    /// the text has actually been measured.
    /// The run screen must say what a whole-book pass is doing. Before this,
    /// expert level showed a 0% bar and nothing else for minutes.
    #[test]
    fn a_whole_book_pass_names_itself_and_clears_when_a_chapter_lands() {
        let mut b = Board::new("book.epub", vec![("Chapter 1".to_string(), 100usize)]);
        b.interactive = false;
        assert!(b.prep.is_none());
        b.prep("reading the whole book").unwrap();
        assert_eq!(b.prep.as_deref(), Some("reading the whole book"));
        b.finish(0, Duration::from_secs(1), true).unwrap();
        assert!(b.prep.is_none(), "a finished chapter must clear the label");
    }

    #[test]
    fn eta_ignores_fixed_cost_phases_and_stays_quiet_on_a_tiny_sample() {
        let chapters = vec![
            ("Cover".to_string(), 11usize),
            ("Title".to_string(), 73),
            ("Contents".to_string(), 98),
            ("Chapter 1".to_string(), 21_900),
            ("Chapter 2".to_string(), 10_200),
        ];
        let mut b = Board::new("book.epub", chapters);
        b.interactive = false;

        // 0.5% of the text done, after a long pre-scan: say nothing.
        b.finish(0, Duration::from_secs(2), true).unwrap();
        b.finish(1, Duration::from_secs(2), true).unwrap();
        b.finish(2, Duration::from_secs(2), true).unwrap();
        assert!(
            b.remaining().is_none(),
            "must not extrapolate the book from its front matter"
        );

        // A real chapter lands: 21,900 chars in 600s → 10,200 left ≈ 279s.
        b.finish(3, Duration::from_secs(600), true).unwrap();
        let left = b
            .remaining()
            .expect("a measured chapter is enough to predict");
        assert!(
            left.as_secs() > 200 && left.as_secs() < 400,
            "expected roughly 5 minutes, got {}",
            fmt_dur(left)
        );
    }
}
