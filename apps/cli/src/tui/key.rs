//! Keyboard input, reduced to a small set of intents.
//!
//! Call sites match on `Key::Up`, never on `'k'` or `\x1b[A`. Two shapes of
//! that mapping exist, because a list you can *search* cannot also treat every
//! letter as a command:
//!
//! * [`Mode::Command`] — vim motions live (`j`/`k`/`h`/`l`, `gg`, `G`), `q`
//!   quits.
//! * [`Mode::Text`] — every printable character is just a character. Without
//!   this you cannot type "quick" into a filter box, because `q` would quit.
//!
//! Escape is layered rather than absolute: in a filter it clears the filter,
//! and only a second press leaves the screen. Destructive-feeling exits should
//! take two deliberate actions.

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Letters are commands.
    Command,
    /// Letters are text.
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Top,
    Bottom,
    Enter,
    Space,
    /// Escape, or an explicit quit key in command mode.
    Quit,
    Backspace,
    /// Ctrl+U — clear the whole field.
    ClearLine,
    /// Ctrl+D — accept a multi-line field, where Enter has to mean newline.
    Accept,
    /// Begin filtering.
    Search,
    Char(char),
    /// Something we do not bind. Callers ignore it; it exists so that an
    /// unrecognised key is a no-op rather than a mis-trigger.
    Other,
}

/// Block until a key is pressed and return its intent.
///
/// `Ctrl+C` is reported as [`Key::Quit`] rather than being swallowed, so every
/// screen unwinds through its normal exit path (restoring the terminal) rather
/// than dying mid-frame.
pub fn read(mode: Mode) -> Result<Key> {
    loop {
        let Event::Key(ev) = event::read()? else {
            continue;
        };
        // Windows reports press *and* release; without this every keystroke
        // moves the cursor twice.
        if ev.kind != KeyEventKind::Press {
            continue;
        }
        if let Some(k) = classify(ev, mode)? {
            return Ok(k);
        }
    }
}

fn classify(ev: KeyEvent, mode: Mode) -> Result<Option<Key>> {
    if ev.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(match ev.code {
            KeyCode::Char('c') => Some(Key::Quit),
            KeyCode::Char('u') => Some(Key::ClearLine),
            KeyCode::Char('d') => Some(Key::Accept),
            _ => Some(Key::Other),
        });
    }

    let k = match ev.code {
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Top,
        KeyCode::End => Key::Bottom,
        KeyCode::Enter => Key::Enter,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Esc => Key::Quit,
        KeyCode::Char(' ') if mode == Mode::Command => Key::Space,
        KeyCode::Char(c) if mode == Mode::Text => Key::Char(c),
        KeyCode::Char(c) => match c {
            'q' | 'Q' => Key::Quit,
            'j' => Key::Down,
            'k' => Key::Up,
            'h' => Key::Left,
            'l' => Key::Right,
            'G' => Key::Bottom,
            '/' => Key::Search,
            // `gg` jumps to the top. A lone `g` is not a binding, so wait
            // briefly for its partner and treat anything else as a no-op.
            'g' => {
                if event::poll(Duration::from_millis(300))? {
                    if let Event::Key(next) = event::read()? {
                        if next.code == KeyCode::Char('g') && next.kind == KeyEventKind::Press {
                            return Ok(Some(Key::Top));
                        }
                    }
                }
                Key::Other
            }
            c => Key::Char(c),
        },
        _ => Key::Other,
    };
    Ok(Some(k))
}

/// Discard anything already sitting in the input buffer.
///
/// Called on both sides of a confirmation prompt. A long scan invites
/// impatient keypresses, and without this the first of them would be consumed
/// the instant the prompt appears — answering a question the user never saw.
pub fn drain() {
    let mut guard = 0;
    while guard < 128 && event::poll(Duration::from_millis(1)).unwrap_or(false) {
        if event::read().is_err() {
            break;
        }
        guard += 1;
    }
}
