//! The four screens every flow is assembled from: a menu, a filterable list,
//! a settings form, and a confirm gate.
//!
//! All of them share one loop shape — draw a full frame, block on a key, act,
//! repeat — and one exit contract: `Ok(None)` means the user backed out, and
//! the caller should unwind rather than proceed.

use super::i18n::{tr, trn};
use super::key::{self, Key, Mode};
use super::term::{self, Frame};
use super::theme::{self, ARROW};
use anyhow::Result;

/// The grey key legend pinned to the bottom of every screen.
///
/// Only ever lists keys that do something *here* — a legend advertising a
/// binding that is inert on the current screen is worse than no legend.
fn legend(keys: &[(&str, &str)]) -> String {
    let parts: Vec<String> = keys.iter().map(|(k, v)| format!("{k} {v}")).collect();
    theme::gray(&parts.join("  ·  "))
}

/// Render one row of a list. `selected` swaps a two-column arrow prefix in for
/// two spaces — same width, so nothing shifts sideways as the cursor moves.
fn row(text: &str, selected: bool) -> String {
    if selected {
        theme::cyan(&format!("{ARROW} {text}"))
    } else {
        format!("  {text}")
    }
}

/// A titled menu with number shortcuts.
pub struct Menu<'a> {
    pub title: &'a str,
    /// `(verb, what it does)`. The verb column is padded to a common width so
    /// the descriptions form a clean second column.
    pub items: &'a [(&'a str, &'a str)],
}

impl Menu<'_> {
    /// Returns the chosen index, or `None` if the user quit.
    ///
    /// Number keys are *direct*: pressing `2` runs the second item
    /// immediately rather than moving the cursor to it. For a five-item menu
    /// used repeatedly, that is the difference between one keystroke and three.
    pub fn run(&self) -> Result<Option<usize>> {
        let mut cur = 0usize;
        let verb_col = self
            .items
            .iter()
            .map(|(v, _)| term::width(v))
            .max()
            .unwrap_or(0)
            + 4;
        loop {
            let (cols, _) = term::size();
            let mut f = Frame::new();
            f.blank();
            for line in theme::banner(cols) {
                f.line(&line);
            }
            f.blank();
            f.line(&theme::purple_bold(self.title));
            f.blank();
            for (i, (verb, desc)) in self.items.iter().enumerate() {
                let text = format!("{}. {}{}", i + 1, term::pad(verb, verb_col), desc);
                f.line(&row(&text, i == cur));
            }
            f.blank();
            f.line(&legend(&[
                ("↑↓", tr("leg.move")),
                ("1-9", tr("leg.jump")),
                ("Enter", tr("leg.select")),
                ("Q", tr("leg.quit")),
            ]));
            f.finish()?;

            match key::read(Mode::Command)? {
                Key::Up => cur = cur.saturating_sub(1),
                Key::Down => cur = (cur + 1).min(self.items.len() - 1),
                Key::Top => cur = 0,
                Key::Bottom => cur = self.items.len() - 1,
                Key::Enter => return Ok(Some(cur)),
                Key::Quit => return Ok(None),
                Key::Char(c) if c.is_ascii_digit() && c != '0' => {
                    let i = c as usize - '1' as usize;
                    if i < self.items.len() {
                        return Ok(Some(i));
                    }
                }
                _ => {}
            }
        }
    }
}

/// One row of a [`List`].
#[derive(Clone)]
pub struct Item {
    /// Shown in the left column.
    pub label: String,
    /// Shown greyed to the right — size, path, whatever qualifies the label.
    pub detail: String,
}

/// A scrolling, filterable single-select list.
///
/// Multi-select is deliberately absent: mole selects many things to delete at
/// once, whereas every list here picks exactly one book, so a checkbox column
/// would be dead weight.
pub struct List<'a> {
    pub title: &'a str,
    pub items: Vec<Item>,
    /// Shown centred when `items` is empty — phrased as a next step, not as a
    /// failure.
    pub empty: &'a str,
    /// Trailing pseudo-entries ("Somewhere else…") that are selectable but are
    /// not things — excluded from the "N books" badge so the count stays honest.
    pub pseudo_tail: usize,
}

impl List<'_> {
    pub fn run(&self) -> Result<Option<usize>> {
        let mut cur = 0usize;
        let mut top = 0usize;
        let mut filter = String::new();
        let mut searching = false;

        loop {
            let (cols, rows) = term::size();
            // title + blank + …rows… + blank + legend, inside a 4-row margin.
            let page = rows.saturating_sub(8).clamp(1, 30);

            let view: Vec<usize> = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, it)| {
                    filter.is_empty()
                        || it.label.to_lowercase().contains(&filter.to_lowercase())
                        || it.detail.to_lowercase().contains(&filter.to_lowercase())
                })
                .map(|(i, _)| i)
                .collect();

            cur = cur.min(view.len().saturating_sub(1));
            if cur < top {
                top = cur;
            } else if cur >= top + page {
                top = cur + 1 - page;
            }

            let mut f = Frame::new();
            f.blank();
            let head = if searching {
                format!(
                    "{}   {}",
                    theme::purple_bold(self.title),
                    theme::yellow(&format!("/ {filter}_")),
                )
            } else {
                theme::purple_bold(self.title)
            };
            let real = self.items.len().saturating_sub(self.pseudo_tail);
            let count = if view.len() == self.items.len() {
                trn("n.book", real)
            } else {
                format!("{}/{}", view.len().min(real), real)
            };
            f.line(&format!("  {head}   {}", theme::gray(&count)));
            f.blank();

            if view.is_empty() {
                f.line(&format!("  {}", theme::gray(self.empty)));
            } else {
                let label_col = view
                    .iter()
                    .map(|&i| term::width(&self.items[i].label))
                    .max()
                    .unwrap_or(20)
                    .min(cols.saturating_sub(28))
                    .max(10);
                for (n, &i) in view.iter().skip(top).take(page).enumerate() {
                    let it = &self.items[i];
                    let label = term::pad(&term::truncate(&it.label, label_col), label_col);
                    let selected = top + n == cur;
                    // The detail column keeps its grey even on the selected
                    // row, so the cyan stays attached to the thing you chose.
                    let text = format!("{label}  {}", theme::gray(&it.detail));
                    f.line(&row(&text, selected));
                }
            }

            f.blank();
            let search_keys = [
                ("type", "filter"),
                ("⌫", "delete"),
                ("^U", tr("leg.clear")),
                ("Enter", tr("leg.open")),
                ("Esc", tr("leg.done")),
            ];
            let browse_keys = [
                ("↑↓", tr("leg.move")),
                ("/", tr("leg.search")),
                ("Enter", tr("leg.open")),
                ("Q", tr("leg.back")),
            ];
            f.line(&legend(if searching {
                &search_keys
            } else {
                &browse_keys[..]
            }));
            f.finish()?;

            let mode = if searching { Mode::Text } else { Mode::Command };
            match key::read(mode)? {
                Key::Up => cur = cur.saturating_sub(1),
                Key::Down => cur = (cur + 1).min(view.len().saturating_sub(1)),
                Key::Top => cur = 0,
                Key::Bottom => cur = view.len().saturating_sub(1),
                Key::Left => {
                    cur = cur.saturating_sub(page);
                }
                Key::Right => cur = (cur + page).min(view.len().saturating_sub(1)),
                Key::Search if !searching => searching = true,
                Key::Enter => {
                    if let Some(&i) = view.get(cur) {
                        return Ok(Some(i));
                    }
                }
                Key::Backspace if searching => {
                    // Emptying the box exits the filter, so holding backspace
                    // always walks you back out rather than trapping you in an
                    // empty search.
                    filter.pop();
                    if filter.is_empty() {
                        searching = false;
                    }
                }
                Key::ClearLine if searching => filter.clear(),
                Key::Char(c) if searching => filter.push(c),
                // Layered escape: clear the filter first, leave on the second.
                Key::Quit if searching => {
                    searching = false;
                    filter.clear();
                }
                Key::Quit => return Ok(None),
                _ => {}
            }
        }
    }
}

/// One editable setting: a label and a ring of choices cycled with ←/→.
/// What a form row actually is.
///
/// A good config screen is not a list of dropdowns — it has
/// toggles, free text, chips and a masked key field. A form that only cycles
/// through choices cannot express it, which is why the CLI used to expose a
/// fraction of the app's settings.
pub enum Kind {
    /// Left/right cycles. The original and still the common case.
    Choice { choices: Vec<String>, value: usize },
    /// Space toggles. Used for the translate / annotate service switches.
    Toggle { on: bool },
    /// Enter opens an editor: single-line, or a paragraph editor when
    /// `multiline` — the reader profile is prose, and a one-line box would
    /// quietly ask for less than the prompt does.
    Text {
        value: String,
        empty: String,
        multiline: bool,
    },
    /// Enter opens a masked editor. `hint` is what to show when one is stored
    /// (never the key itself).
    Secret { hint: Option<String> },
    /// Enter opens a checkbox list.
    Multi {
        options: Vec<(String, String)>,
        chosen: Vec<bool>,
        empty: String,
    },
    /// Enter runs it. `status` is the last result, shown beside the label.
    Action { status: String },
    /// A non-selectable heading, so one form can carry grouped sections the
    /// way the app's service cards do.
    Header,
}

pub struct Field {
    pub label: String,
    pub kind: Kind,
    /// One grey line under the row when it is selected. Use it for the
    /// consequence of the current choice, not for a restatement of the label.
    pub help: String,
    /// Rendered grey and skipped by the cursor. A section whose service is
    /// switched off greys out rather than disappearing, so the screen does not
    /// reflow under the user.
    pub disabled: bool,
}

impl Field {
    pub fn choice(label: &str, choices: &[&str], value: usize, help: &str) -> Self {
        Field {
            label: label.into(),
            kind: Kind::Choice {
                choices: choices.iter().map(|c| c.to_string()).collect(),
                value,
            },
            help: help.into(),
            disabled: false,
        }
    }
    pub fn toggle(label: &str, on: bool, help: &str) -> Self {
        Field {
            label: label.into(),
            kind: Kind::Toggle { on },
            help: help.into(),
            disabled: false,
        }
    }
    pub fn paragraph(label: &str, value: &str, empty: &str, help: &str) -> Self {
        let mut f = Field::text(label, value, empty, help);
        if let Kind::Text { multiline, .. } = &mut f.kind {
            *multiline = true;
        }
        f
    }
    pub fn text(label: &str, value: &str, empty: &str, help: &str) -> Self {
        Field {
            label: label.into(),
            kind: Kind::Text {
                value: value.into(),
                empty: empty.into(),
                multiline: false,
            },
            help: help.into(),
            disabled: false,
        }
    }
    pub fn secret(label: &str, hint: Option<String>, help: &str) -> Self {
        Field {
            label: label.into(),
            kind: Kind::Secret { hint },
            help: help.into(),
            disabled: false,
        }
    }
    pub fn multi(
        label: &str,
        options: Vec<(String, String)>,
        chosen: Vec<bool>,
        empty: &str,
        help: &str,
    ) -> Self {
        Field {
            label: label.into(),
            kind: Kind::Multi {
                options,
                chosen,
                empty: empty.into(),
            },
            help: help.into(),
            disabled: false,
        }
    }
    pub fn action(label: &str, help: &str) -> Self {
        Field {
            label: label.into(),
            kind: Kind::Action {
                status: String::new(),
            },
            help: help.into(),
            disabled: false,
        }
    }
    pub fn header(label: &str) -> Self {
        Field {
            label: label.into(),
            kind: Kind::Header,
            help: String::new(),
            disabled: false,
        }
    }

    /// The value column, as displayed.
    pub fn shown(&self) -> String {
        match &self.kind {
            Kind::Choice { choices, value } => choices[*value].clone(),
            Kind::Toggle { on } => {
                if *on {
                    "on".into()
                } else {
                    "off".into()
                }
            }
            Kind::Text { value, empty, .. } => {
                if value.trim().is_empty() {
                    empty.clone()
                } else {
                    term::truncate(value, 46)
                }
            }
            Kind::Secret { hint } => hint.clone().unwrap_or_else(|| tr("set.key.notset").into()),
            Kind::Multi {
                options,
                chosen,
                empty,
            } => {
                let picked: Vec<&str> = options
                    .iter()
                    .zip(chosen.iter())
                    .filter(|(_, c)| **c)
                    .map(|(o, _)| o.0.as_str())
                    .collect();
                if picked.is_empty() {
                    empty.clone()
                } else {
                    term::truncate(&picked.join(", "), 46)
                }
            }
            Kind::Action { status } => status.clone(),
            Kind::Header => String::new(),
        }
    }

    pub fn current(&self) -> &str {
        match &self.kind {
            Kind::Choice { choices, value } => &choices[*value],
            _ => "",
        }
    }
    pub fn is_on(&self) -> bool {
        matches!(self.kind, Kind::Toggle { on: true })
    }
    pub fn text_value(&self) -> String {
        match &self.kind {
            Kind::Text { value, .. } => value.clone(),
            _ => String::new(),
        }
    }
    pub fn picked(&self) -> Vec<String> {
        match &self.kind {
            Kind::Multi {
                options, chosen, ..
            } => options
                .iter()
                .zip(chosen.iter())
                .filter(|(_, c)| **c)
                .map(|(o, _)| o.0.clone())
                .collect(),
            _ => Vec::new(),
        }
    }
    fn selectable(&self) -> bool {
        !matches!(self.kind, Kind::Header) && !self.disabled
    }
}

/// What a form run ended with.
pub enum FormExit {
    /// Enter on the submit row, or Enter anywhere with no submit row.
    Submit,
    /// Esc / Q.
    Back,
    /// An `Action` field was triggered; the caller handles it and re-enters.
    Action(usize),
}

/// A settings form.
///
/// Rows are heterogeneous (see [`Kind`]) and the interaction adapts per row:
/// left/right cycles a choice, Space flips a toggle, Enter opens an editor for
/// text, secrets and chip lists. That per-row dispatch is the whole point — the
/// app's config screen mixes all of these on one page, and a form that only did
/// dropdowns is why the CLI exposed a fraction of it.
///
/// `submit` is the label of a final row that means "go". Without one, Enter on
/// any non-interactive row submits.
pub fn form(
    title: &str,
    subtitle: &str,
    fields: &mut [Field],
    submit: Option<&str>,
) -> Result<FormExit> {
    form_with(title, subtitle, fields, submit, &mut |_| {})
}

/// `form`, plus a hook run before every repaint.
///
/// The hook exists for cross-field state: switching a service off has to grey
/// its section *now*, not the next time the screen is entered. Without it,
/// toggling "Add notes" on and pressing down skipped the note fields entirely,
/// because their disabled flags were still whatever they were on entry.
pub fn form_with(
    title: &str,
    subtitle: &str,
    fields: &mut [Field],
    submit: Option<&str>,
    refresh: &mut dyn FnMut(&mut [Field]),
) -> Result<FormExit> {
    let mut cur = fields.iter().position(|f| f.selectable()).unwrap_or(0);
    // First visible row. The config screen is taller than a 24-line terminal,
    // and a frame that overflows scrolls the title away and defeats the
    // clear-as-you-go redraw — so the rows get a viewport rather than the
    // terminal getting a scrollback.
    let mut top = 0usize;
    loop {
        refresh(fields);
        // A refresh can disable the row the cursor is on; move off it rather
        // than leaving the cursor parked somewhere it cannot act.
        if cur < fields.len() && !fields[cur].selectable() {
            cur = fields
                .iter()
                .enumerate()
                .skip(cur)
                .find(|(_, f)| f.selectable())
                .map(|(i, _)| i)
                .or_else(|| fields.iter().position(|f| f.selectable()))
                .unwrap_or(cur);
        }
        let label_col = fields
            .iter()
            .filter(|f| !matches!(f.kind, Kind::Header))
            .map(|f| term::width(&f.label))
            .max()
            .unwrap_or(0)
            + 3;
        let submit_row = fields.len();
        let last = if submit.is_some() {
            submit_row
        } else {
            fields.len() - 1
        };

        // Rows the chrome always needs: two blanks, title, optional subtitle,
        // the help line under the cursor, the submit row and its blank, the
        // legend and its blank, plus the two scroll markers.
        let (_, rows) = term::size();
        let chrome =
            4 + usize::from(!subtitle.is_empty()) + usize::from(submit.is_some()) * 2 + 2 + 2;
        let window = rows.saturating_sub(chrome).max(4);

        // Keep the cursor inside the window, and never leave a gap at the end.
        if cur < top {
            top = cur;
        } else if cur >= top + window && cur < fields.len() {
            top = cur + 1 - window;
        }
        if fields.len() > window {
            top = top.min(fields.len() - window);
        } else {
            top = 0;
        }
        let end = (top + window).min(fields.len());

        let mut f = Frame::new();
        f.blank();
        f.line(&format!("  {}", theme::purple_bold(title)));
        if !subtitle.is_empty() {
            f.line(&format!("  {}", theme::gray(subtitle)));
        }
        f.blank();
        if top > 0 {
            f.line(&theme::gray(&format!("  ↑ {top} more above")));
        }
        for (i, fl) in fields.iter().enumerate().take(end).skip(top) {
            let selected = i == cur;
            if matches!(fl.kind, Kind::Header) {
                f.line(&format!("  {}", theme::blue(&fl.label)));
                continue;
            }
            let shown = fl.shown();
            // Arrow hints only on the active row, and only where left/right
            // actually does something — advertising ‹ › on a text field would
            // be a lie.
            let val = match (&fl.kind, selected) {
                (Kind::Choice { .. }, true) => format!("‹ {shown} ›"),
                (_, true) => format!("  {shown}"),
                _ => format!("  {shown}"),
            };
            let text = format!("{}{}", term::pad(&fl.label, label_col), val);
            let line = if fl.disabled {
                theme::gray(&format!("  {text}"))
            } else {
                row(&text, selected)
            };
            f.line(&line);
            if selected && !fl.help.is_empty() {
                f.line(&format!("     {}", theme::gray(&fl.help)));
            }
        }
        if end < fields.len() {
            f.line(&theme::gray(&format!(
                "  ↓ {} more below",
                fields.len() - end
            )));
        }
        if let Some(label) = submit {
            f.blank();
            // No literal arrow here: `row` supplies the cursor marker, and
            // baking one in renders "➤ ➤ Continue" on the selected row.
            f.line(&row(label, cur == submit_row));
        }
        f.blank();
        // The legend names only what the row under the cursor can do.
        let verb = fields.get(cur).map(|fl| match fl.kind {
            Kind::Choice { .. } => ("←→", tr("leg.change")),
            Kind::Toggle { .. } => ("Space", tr("leg.toggle")),
            Kind::Text { .. } | Kind::Secret { .. } => ("Enter", tr("leg.edit")),
            Kind::Multi { .. } => ("Enter", tr("leg.choose")),
            Kind::Action { .. } => ("Enter", tr("leg.run")),
            Kind::Header => ("", ""),
        });
        let mut keys: Vec<(&str, &str)> = vec![("↑↓", tr("leg.field"))];
        if let Some((k, v)) = verb {
            if !k.is_empty() && cur != submit_row {
                keys.push((k, v));
            }
        }
        if submit.is_some() {
            keys.push(("Enter", tr("leg.onarrow")));
        } else {
            keys.push(("Enter", tr("leg.continue")));
        }
        keys.push(("Esc", tr("leg.back")));
        f.line(&legend(&keys));
        f.finish()?;

        let move_to = |from: usize, delta: isize, fields: &[Field]| -> usize {
            let n = if submit.is_some() {
                fields.len() + 1
            } else {
                fields.len()
            };
            let mut i = from as isize;
            for _ in 0..n {
                i += delta;
                if i < 0 {
                    i = n as isize - 1;
                } else if i >= n as isize {
                    i = 0;
                }
                let idx = i as usize;
                if idx == fields.len() || fields[idx].selectable() {
                    return idx;
                }
            }
            from
        };

        match key::read(Mode::Command)? {
            Key::Up => cur = move_to(cur, -1, fields),
            Key::Down => cur = move_to(cur, 1, fields),
            Key::Top => cur = fields.iter().position(|f| f.selectable()).unwrap_or(0),
            Key::Bottom => cur = last,
            Key::Quit => return Ok(FormExit::Back),
            Key::Left if cur < fields.len() => {
                if let Kind::Choice { choices, value } = &mut fields[cur].kind {
                    *value = if *value == 0 {
                        choices.len() - 1
                    } else {
                        *value - 1
                    };
                }
            }
            Key::Right if cur < fields.len() => {
                if let Kind::Choice { choices, value } = &mut fields[cur].kind {
                    *value = (*value + 1) % choices.len();
                }
            }
            Key::Space if cur < fields.len() => {
                if let Kind::Toggle { on } = &mut fields[cur].kind {
                    *on = !*on;
                }
            }
            Key::Enter => {
                if submit.is_some() && cur == submit_row {
                    return Ok(FormExit::Submit);
                }
                if cur >= fields.len() {
                    return Ok(FormExit::Submit);
                }
                match &mut fields[cur].kind {
                    Kind::Text {
                        value, multiline, ..
                    } => {
                        let label = fields[cur].label.clone();
                        let help = fields[cur].help.clone();
                        let initial = value.clone();
                        let long = *multiline;
                        let edited = if long {
                            textarea(&label, &help, &initial, &[])?
                        } else {
                            input(&label, &help, &initial, false, &[])?
                        };
                        if let Some(v) = edited {
                            if let Kind::Text { value, .. } = &mut fields[cur].kind {
                                *value = v;
                            }
                        }
                    }
                    Kind::Secret { .. } => return Ok(FormExit::Action(cur)),
                    Kind::Multi {
                        options, chosen, ..
                    } => {
                        let label = fields[cur].label.clone();
                        let help = fields[cur].help.clone();
                        let opts = options.clone();
                        let mut pick = chosen.clone();
                        if multi(&label, &help, &opts, &mut pick)? {
                            if let Kind::Multi { chosen, .. } = &mut fields[cur].kind {
                                *chosen = pick;
                            }
                        }
                    }
                    Kind::Action { .. } => return Ok(FormExit::Action(cur)),
                    // Choice and Toggle rows have their own keys. Submitting
                    // from them would make Enter mean two different things
                    // depending on where the cursor happens to be.
                    _ => {
                        if submit.is_none() {
                            return Ok(FormExit::Submit);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// The gate in front of anything that costs money or time.
///
/// `consequence` must be quantified — "42 chapters, ~118k words, ~38 min,
/// ~$1.20", never "Proceed?". The user is agreeing to a specific bill, and a
/// prompt that hides the number is asking them to agree to nothing.
///
/// Enter confirms; **every other key cancels**. Cancelling is free, so it is
/// the safe default for a mistyped keystroke.
pub fn confirm(lines: &[String], consequence: &str) -> Result<bool> {
    key::drain();
    let mut f = Frame::new();
    f.blank();
    for l in lines {
        f.line(l);
    }
    f.blank();
    f.line(&format!(
        "  {} {}  {} {}, {} {}",
        theme::purple(ARROW),
        consequence,
        theme::green("Enter"),
        tr("leg.confirm"),
        theme::gray("Esc"),
        tr("leg.cancel")
    ));
    f.finish()?;

    let k = key::read(Mode::Command)?;
    key::drain();
    Ok(k == Key::Enter)
}

/// A full-screen message with a single acknowledgement.
pub fn notice(title: &str, body: &[String]) -> Result<()> {
    let mut f = Frame::new();
    f.blank();
    f.line(&format!("  {}", theme::purple_bold(title)));
    f.blank();
    for l in body {
        f.line(l);
    }
    f.blank();
    f.line(&legend(&[("Enter", tr("leg.continue"))]));
    f.finish()?;
    loop {
        match key::read(Mode::Command)? {
            Key::Enter | Key::Quit => return Ok(()),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Text entry
// ---------------------------------------------------------------------------

/// A single-line editor. Returns `None` if the user cancelled.
///
/// `masked` renders the value as bullets — used for API keys, which should not
/// survive on screen or in a screenshot. The value itself is still returned in
/// the clear; masking is a display decision, not storage.
///
/// Keys are read in `Mode::Text`, so every printable character is a character:
/// without that, typing "quick" into a profile would quit the screen on the q.
pub fn input(
    title: &str,
    prompt: &str,
    initial: &str,
    masked: bool,
    help: &[&str],
) -> Result<Option<String>> {
    let mut buf: Vec<char> = initial.chars().collect();
    let mut cur = buf.len();
    loop {
        let shown: String = if masked {
            "•".repeat(buf.len())
        } else {
            buf.iter().collect()
        };
        let mut f = Frame::new();
        f.blank();
        f.line(&format!("  {}", theme::purple_bold(title)));
        f.blank();
        f.line(&format!("  {}", theme::gray(prompt)));
        f.blank();
        // The caret sits between characters, drawn as a block on the character
        // it precedes so the position is unambiguous at the end of the line.
        let (before, after): (String, String) = if masked {
            (shown.clone(), String::new())
        } else {
            (buf[..cur].iter().collect(), buf[cur..].iter().collect())
        };
        f.line(&format!(
            "  {} {}{}{}",
            theme::purple(ARROW),
            before,
            theme::cyan("▏"),
            after
        ));
        f.blank();
        for h in help {
            f.line(&format!("  {}", theme::gray(h)));
        }
        if !help.is_empty() {
            f.blank();
        }
        f.line(&legend(&[
            ("Enter", tr("leg.save")),
            ("Esc", tr("leg.cancel")),
            ("Ctrl+U", tr("leg.clear")),
        ]));
        f.finish()?;

        match key::read(Mode::Text)? {
            Key::Enter => return Ok(Some(buf.iter().collect())),
            Key::Quit => return Ok(None),
            Key::ClearLine => {
                buf.clear();
                cur = 0;
            }
            Key::Backspace => {
                if cur > 0 {
                    buf.remove(cur - 1);
                    cur -= 1;
                }
            }
            Key::Left => cur = cur.saturating_sub(1),
            Key::Right => cur = (cur + 1).min(buf.len()),
            Key::Top => cur = 0,
            Key::Bottom => cur = buf.len(),
            Key::Char(c) => {
                buf.insert(cur, c);
                cur += 1;
            }
            _ => {}
        }
    }
}

/// A multi-line free-text editor, for the reader profile.
///
/// The profile is a paragraph, not a word: forcing it onto one line would make
/// people write less than the prompt asks for. Enter inserts a newline here and
/// Ctrl+D accepts, which is the inverse of `input` — the legend says so, because
/// nothing about it is guessable.
pub fn textarea(title: &str, prompt: &str, initial: &str, help: &[&str]) -> Result<Option<String>> {
    let mut lines: Vec<Vec<char>> = if initial.is_empty() {
        vec![Vec::new()]
    } else {
        initial.lines().map(|l| l.chars().collect()).collect()
    };
    let mut row = lines.len() - 1;
    let mut col = lines[row].len();
    loop {
        let mut f = Frame::new();
        f.blank();
        f.line(&format!("  {}", theme::purple_bold(title)));
        f.blank();
        f.line(&format!("  {}", theme::gray(prompt)));
        f.blank();
        for (i, l) in lines.iter().enumerate() {
            let text: String = l.iter().collect();
            if i == row {
                let before: String = l[..col].iter().collect();
                let after: String = l[col..].iter().collect();
                f.line(&format!("  {before}{}{after}", theme::cyan("▏")));
            } else {
                f.line(&format!("  {text}"));
            }
        }
        f.blank();
        for h in help {
            f.line(&format!("  {}", theme::gray(h)));
        }
        if !help.is_empty() {
            f.blank();
        }
        f.line(&legend(&[
            ("Ctrl+D", tr("leg.save")),
            ("Enter", tr("leg.newline")),
            ("Esc", tr("leg.cancel")),
        ]));
        f.finish()?;

        match key::read(Mode::Text)? {
            Key::Accept => {
                let out: Vec<String> = lines.iter().map(|l| l.iter().collect()).collect();
                return Ok(Some(out.join("\n").trim().to_string()));
            }
            Key::Quit => return Ok(None),
            Key::Enter => {
                let tail = lines[row].split_off(col);
                lines.insert(row + 1, tail);
                row += 1;
                col = 0;
            }
            Key::Backspace => {
                if col > 0 {
                    lines[row].remove(col - 1);
                    col -= 1;
                } else if row > 0 {
                    let cur_line = lines.remove(row);
                    row -= 1;
                    col = lines[row].len();
                    lines[row].extend(cur_line);
                }
            }
            Key::Up if row > 0 => {
                row -= 1;
                col = col.min(lines[row].len());
            }
            Key::Down if row + 1 < lines.len() => {
                row += 1;
                col = col.min(lines[row].len());
            }
            Key::Left => col = col.saturating_sub(1),
            Key::Right => col = (col + 1).min(lines[row].len()),
            Key::ClearLine => {
                lines[row].clear();
                col = 0;
            }
            Key::Char(c) => {
                lines[row].insert(col, c);
                col += 1;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Multi-select
// ---------------------------------------------------------------------------

/// A checkbox list. `chosen` is edited in place; returns false if cancelled.
///
/// Uses mole's `○`/`●` pair rather than `[ ]`/`[x]`: both are one column wide,
/// so the label column never shifts as things are ticked.
pub fn multi(
    title: &str,
    subtitle: &str,
    options: &[(String, String)],
    chosen: &mut [bool],
) -> Result<bool> {
    let mut cur = 0usize;
    let label_col = options
        .iter()
        .map(|(l, _)| term::width(l))
        .max()
        .unwrap_or(0)
        + 3;
    loop {
        let mut f = Frame::new();
        f.blank();
        f.line(&format!("  {}", theme::purple_bold(title)));
        if !subtitle.is_empty() {
            f.line(&format!("  {}", theme::gray(subtitle)));
        }
        f.blank();
        for (i, (label, help)) in options.iter().enumerate() {
            let mark = if chosen[i] { "●" } else { "○" };
            let text = format!(
                "{mark} {}{}",
                term::pad(label, label_col),
                theme::gray(help)
            );
            f.line(&row(&text, i == cur));
        }
        f.blank();
        let n = chosen.iter().filter(|c| **c).count();
        f.line(&format!(
            "  {}",
            theme::gray(&super::i18n::tr1(
                "multi.selected",
                &format!("{n}/{}", options.len())
            ))
        ));
        f.blank();
        f.line(&legend(&[
            ("↑↓", tr("leg.move")),
            ("Space", tr("leg.toggle")),
            ("Enter", tr("leg.done")),
            ("Esc", tr("leg.back")),
        ]));
        f.finish()?;

        match key::read(Mode::Command)? {
            Key::Up => cur = if cur == 0 { options.len() - 1 } else { cur - 1 },
            Key::Down => cur = (cur + 1) % options.len(),
            Key::Space => chosen[cur] = !chosen[cur],
            Key::Enter => return Ok(true),
            Key::Quit => return Ok(false),
            Key::Char(c) if c.is_ascii_digit() => {
                let i = c.to_digit(10).unwrap_or(0) as usize;
                if i >= 1 && i <= options.len() {
                    chosen[i - 1] = !chosen[i - 1];
                    cur = i - 1;
                }
            }
            _ => {}
        }
    }
}
