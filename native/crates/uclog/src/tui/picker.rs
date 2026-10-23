//! In-TUI file picker: suggestions, a directory listing, and a typed path with Tab completion
//! (TUI plan §3, README §5.5).

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};

use super::lineedit::{Edit, LineEdit};

pub struct Picker {
    pub title: String,
    /// Extensions shown (lowercase, without the dot); empty = all files.
    exts: Vec<String>,
    pub dir: PathBuf,
    pub suggestions: Vec<PathBuf>,
    /// Suggestions first, then `..`, directories, files.
    pub entries: Vec<(String, PathBuf, bool)>,
    pub selected: usize,
    pub path: LineEdit,
    pub error: Option<String>,
}

pub enum Pick {
    None,
    Chosen(PathBuf),
    Cancel,
}

impl Picker {
    pub fn new(title: &str, exts: &[&str], dir: PathBuf, suggestions: Vec<PathBuf>) -> Self {
        let mut p = Picker {
            title: title.into(),
            exts: exts.iter().map(|e| e.to_string()).collect(),
            dir,
            suggestions,
            entries: vec![],
            selected: 0,
            path: LineEdit::default(),
            error: None,
        };
        p.refresh();
        p
    }

    fn wanted(&self, p: &Path) -> bool {
        self.exts.is_empty()
            || p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| self.exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
    }

    pub fn refresh(&mut self) {
        let mut entries: Vec<(String, PathBuf, bool)> =
            self.suggestions.iter().map(|s| (format!("★ {}", s.display()), s.clone(), false)).collect();
        if let Some(parent) = self.dir.parent() {
            entries.push(("../".into(), parent.to_path_buf(), true));
        }
        let mut dirs = vec![];
        let mut files = vec![];
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let p = e.path();
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    if p.is_dir() {
                        dirs.push((format!("{name}/"), p, true));
                    } else if self.wanted(&p) {
                        files.push((name, p, false));
                    }
                }
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{}: {e}", self.dir.display())),
        }
        dirs.sort();
        files.sort();
        entries.extend(dirs);
        entries.extend(files);
        self.entries = entries;
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    fn open(&mut self, p: PathBuf, is_dir: bool) -> Pick {
        if is_dir {
            self.dir = p;
            self.selected = 0;
            self.refresh();
            Pick::None
        } else {
            Pick::Chosen(p)
        }
    }

    /// Complete the typed path to the longest common prefix of matching names.
    fn complete(&mut self) {
        let typed = self.path.text.clone();
        let full = if Path::new(&typed).is_absolute() { PathBuf::from(&typed) } else { self.dir.join(&typed) };
        let (dir, stem) = if typed.ends_with('/') || typed.is_empty() {
            (full.clone(), String::new())
        } else {
            (
                full.parent().map(Path::to_path_buf).unwrap_or_default(),
                full.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            )
        };
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let names: Vec<String> = rd
            .flatten()
            .map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                if e.path().is_dir() {
                    format!("{n}/")
                } else {
                    n
                }
            })
            .filter(|n| n.starts_with(&stem))
            .collect();
        let Some(first) = names.first() else { return };
        let mut common = first.clone();
        for n in &names[1..] {
            while !n.starts_with(&common) {
                common.pop();
            }
        }
        if common.len() > stem.len() {
            let add = &common[stem.len()..];
            let t = format!("{typed}{add}");
            self.path.set(&t);
        }
    }

    pub fn key(&mut self, k: KeyEvent) -> Pick {
        match k.code {
            KeyCode::Esc => return Pick::Cancel,
            KeyCode::Tab => {
                self.complete();
                return Pick::None;
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                return Pick::None;
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
                return Pick::None;
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                return Pick::None;
            }
            KeyCode::PageDown => {
                self.selected = (self.selected + 10).min(self.entries.len().saturating_sub(1));
                return Pick::None;
            }
            _ => {}
        }
        match self.path.key(k) {
            Edit::Submit(t) if !t.trim().is_empty() => {
                let t = shellexpand_home(t.trim());
                let p = if Path::new(&t).is_absolute() { PathBuf::from(&t) } else { self.dir.join(&t) };
                if p.is_dir() {
                    self.open(p, true)
                } else if p.exists() {
                    Pick::Chosen(p)
                } else {
                    self.error = Some(format!("{}: not found", p.display()));
                    Pick::None
                }
            }
            Edit::Submit(_) => match self.entries.get(self.selected).cloned() {
                Some((_, p, d)) => self.open(p, d),
                None => Pick::None,
            },
            _ => Pick::None,
        }
    }
}

fn shellexpand_home(s: &str) -> String {
    match (s.strip_prefix("~/"), directories::BaseDirs::new()) {
        (Some(rest), Some(b)) => b.home_dir().join(rest).to_string_lossy().into_owned(),
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::front::signed_images;
    use crossterm::event::KeyModifiers;

    #[test]
    fn browse_complete_choose() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("build/zephyr")).unwrap();
        std::fs::write(d.path().join("build/zephyr/zephyr.signed.bin"), b"x").unwrap();
        std::fs::write(d.path().join("app.bin"), b"x").unwrap();
        std::fs::write(d.path().join("notes.txt"), b"x").unwrap();
        let sug = signed_images(d.path(), 3);
        assert_eq!(sug.len(), 1);
        let mut p = Picker::new("FW image", &["bin"], d.path().into(), sug);
        let names: Vec<_> = p.entries.iter().map(|e| e.0.clone()).collect();
        assert!(names[0].starts_with("★ "));
        assert!(names.contains(&"app.bin".to_string()) && !names.contains(&"notes.txt".to_string()));
        let k = |c| KeyEvent::new(c, KeyModifiers::NONE);
        for c in "bu".chars() {
            p.key(k(KeyCode::Char(c)));
        }
        p.key(k(KeyCode::Tab));
        assert_eq!(p.path.text, "build/");
        for c in "zephyr/zephyr.signed.bin".chars() {
            p.key(k(KeyCode::Char(c)));
        }
        assert!(matches!(p.key(k(KeyCode::Enter)), Pick::Chosen(x) if x.ends_with("zephyr.signed.bin")));
    }
}
