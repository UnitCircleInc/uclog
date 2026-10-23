//! Single-line text input with cursor and history.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Default, Clone)]
pub struct LineEdit {
    pub text: String,
    /// Cursor position in chars.
    pub cursor: usize,
    history: Vec<String>,
    /// Position while browsing history (`history.len()` = the line being edited).
    hpos: usize,
    draft: String,
}

pub enum Edit {
    /// The key changed (or did not apply to) the line.
    Consumed,
    /// Enter: the line, now cleared.
    Submit(String),
    /// Not an editing key.
    Ignored,
}

impl LineEdit {
    pub fn with_history(history: Vec<String>) -> Self {
        let hpos = history.len();
        LineEdit { history, hpos, ..Default::default() }
    }

    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = self.text.chars().count();
    }

    fn byte_at(&self, c: usize) -> usize {
        self.text.char_indices().nth(c).map(|(i, _)| i).unwrap_or(self.text.len())
    }

    pub fn push_history(&mut self, s: &str) {
        if !s.trim().is_empty() {
            self.history.retain(|h| h != s);
            self.history.push(s.to_string());
        }
        self.hpos = self.history.len();
    }

    pub fn key(&mut self, k: KeyEvent) -> Edit {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Enter => {
                let s = std::mem::take(&mut self.text);
                self.cursor = 0;
                self.hpos = self.history.len();
                return Edit::Submit(s);
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.text.chars().count(),
            KeyCode::Char('u') if ctrl => {
                self.text.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                let b = self.byte_at(self.cursor);
                self.text.insert(b, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let b = self.byte_at(self.cursor - 1);
                self.text.remove(b);
                self.cursor -= 1;
            }
            KeyCode::Delete if self.cursor < self.text.chars().count() => {
                let b = self.byte_at(self.cursor);
                self.text.remove(b);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.text.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            KeyCode::Up if self.hpos > 0 => {
                if self.hpos == self.history.len() {
                    self.draft = self.text.clone();
                }
                self.hpos -= 1;
                let h = self.history[self.hpos].clone();
                self.set(&h);
            }
            KeyCode::Down if self.hpos < self.history.len() => {
                self.hpos += 1;
                let h =
                    if self.hpos == self.history.len() { self.draft.clone() } else { self.history[self.hpos].clone() };
                self.set(&h);
            }
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Up | KeyCode::Down => {}
            _ => return Edit::Ignored,
        }
        Edit::Consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn edit_and_history() {
        let mut e = LineEdit::with_history(vec!["one".into(), "two".into()]);
        for c in "héllo".chars() {
            e.key(k(KeyCode::Char(c)));
        }
        e.key(k(KeyCode::Left));
        e.key(k(KeyCode::Backspace));
        assert_eq!(e.text, "hélo");
        e.key(k(KeyCode::Up));
        assert_eq!(e.text, "two");
        e.key(k(KeyCode::Up));
        e.key(k(KeyCode::Up));
        assert_eq!(e.text, "one");
        e.key(k(KeyCode::Down));
        e.key(k(KeyCode::Down));
        assert_eq!(e.text, "hélo");
        assert!(matches!(e.key(k(KeyCode::Enter)), Edit::Submit(s) if s == "hélo"));
        assert!(e.text.is_empty());
    }
}
