//! Terminal view model (port 61): a `vte` parser feeding a scrollback of styled lines. A VT100
//! subset as devices use it: SGR colours, CR/LF/BS/TAB, cursor forward/back/column, erase line
//! and clear screen (GUI plan §2). There is no fixed screen: lines only scroll.

use std::collections::VecDeque;

pub const LINES_MAX: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    /// 0–255: the 16 ANSI colours, then the 256-colour palette.
    Idx(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub inverse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub c: char,
    pub style: Style,
}

#[derive(Default)]
struct Screen {
    lines: VecDeque<Vec<Cell>>,
    col: usize,
    style: Style,
    /// Bumped on every change (the view re-lays out only when it moves).
    version: u64,
}

pub struct Terminal {
    parser: vte::Parser,
    screen: Screen,
}

impl Default for Terminal {
    fn default() -> Self {
        let mut screen = Screen::default();
        screen.lines.push_back(vec![]);
        Terminal { parser: vte::Parser::new(), screen }
    }
}

impl Terminal {
    pub fn feed(&mut self, data: &[u8]) {
        self.parser.advance(&mut self.screen, data);
        self.screen.version += 1;
    }

    pub fn lines(&self) -> &VecDeque<Vec<Cell>> {
        &self.screen.lines
    }

    pub fn clear(&mut self) {
        self.screen.lines.clear();
        self.screen.lines.push_back(vec![]);
        self.screen.col = 0;
        self.screen.version += 1;
    }

    /// Plain text (copy, export).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for (i, l) in self.screen.lines.iter().enumerate() {
            if i > 0 {
                s.push('\n');
            }
            s.extend(l.iter().map(|c| c.c));
        }
        s
    }
}

impl Screen {
    fn line(&mut self) -> &mut Vec<Cell> {
        self.lines.back_mut().expect("there is always a line")
    }

    fn newline(&mut self) {
        self.lines.push_back(vec![]);
        while self.lines.len() > LINES_MAX {
            self.lines.pop_front();
        }
        self.col = 0;
    }

    fn put(&mut self, c: char) {
        let (col, style) = (self.col, self.style);
        let line = self.line();
        while line.len() < col {
            line.push(Cell { c: ' ', style: Style::default() });
        }
        if col < line.len() {
            line[col] = Cell { c, style };
        } else {
            line.push(Cell { c, style });
        }
        self.col += 1;
    }

    fn sgr(&mut self, params: &vte::Params) {
        let p: Vec<u16> = params.iter().flat_map(|s| s.iter().copied()).collect();
        if p.is_empty() {
            self.style = Style::default();
            return;
        }
        let mut i = 0;
        while i < p.len() {
            match p[i] {
                0 => self.style = Style::default(),
                1 => self.style.bold = true,
                7 => self.style.inverse = true,
                22 => self.style.bold = false,
                27 => self.style.inverse = false,
                n @ 30..=37 => self.style.fg = Color::Idx((n - 30) as u8),
                n @ 90..=97 => self.style.fg = Color::Idx((n - 90 + 8) as u8),
                39 => self.style.fg = Color::Default,
                n @ 40..=47 => self.style.bg = Color::Idx((n - 40) as u8),
                n @ 100..=107 => self.style.bg = Color::Idx((n - 100 + 8) as u8),
                49 => self.style.bg = Color::Default,
                n @ (38 | 48) => {
                    // 38;5;N or 38;2;R;G;B
                    let c = match p.get(i + 1) {
                        Some(5) => {
                            let c = p.get(i + 2).map(|&v| Color::Idx(v as u8));
                            i += 2;
                            c
                        }
                        Some(2) => {
                            let g = |k: usize| p.get(i + k).copied().unwrap_or(0) as u8;
                            let c = Some(Color::Rgb(g(2), g(3), g(4)));
                            i += 4;
                            c
                        }
                        _ => None,
                    };
                    if let Some(c) = c {
                        if n == 38 {
                            self.style.fg = c;
                        } else {
                            self.style.bg = c;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

fn arg(params: &vte::Params, default: usize) -> usize {
    match params.iter().next().and_then(|s| s.first().copied()) {
        Some(0) | None => default,
        Some(n) => n as usize,
    }
}

impl vte::Perform for Screen {
    fn print(&mut self, c: char) {
        self.put(c);
    }

    fn execute(&mut self, b: u8) {
        match b {
            b'\n' => self.newline(),
            b'\r' => self.col = 0,
            0x08 => self.col = self.col.saturating_sub(1),
            b'\t' => {
                let next = (self.col / 8 + 1) * 8;
                while self.col < next {
                    self.put(' ');
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &vte::Params, _intermediates: &[u8], _ignore: bool, action: char) {
        match action {
            'm' => self.sgr(params),
            'C' => self.col += arg(params, 1),
            'D' => self.col = self.col.saturating_sub(arg(params, 1)),
            'G' => self.col = arg(params, 1) - 1,
            'K' => {
                let col = self.col;
                let line = self.line();
                match arg(params, 0) {
                    0 => line.truncate(col),
                    1 => {
                        for c in line.iter_mut().take(col + 1) {
                            c.c = ' ';
                        }
                    }
                    _ => line.clear(),
                }
            }
            'J' if arg(params, 0) >= 2 => {
                self.lines.clear();
                self.lines.push_back(vec![]);
                self.col = 0;
            }
            _ => {}
        }
    }
}

/// RGB of an ANSI/xterm-256 colour index.
pub fn palette(i: u8) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 49, 49),
        (13, 188, 121),
        (229, 229, 16),
        (36, 114, 200),
        (188, 63, 188),
        (17, 168, 205),
        (229, 229, 229),
        (102, 102, 102),
        (241, 76, 76),
        (35, 209, 139),
        (245, 245, 67),
        (59, 142, 234),
        (214, 112, 214),
        (41, 184, 219),
        (255, 255, 255),
    ];
    match i {
        0..=15 => BASE[i as usize],
        16..=231 => {
            let v = i - 16;
            let s = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            (s(v / 36), s(v / 6 % 6), s(v % 6))
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            (g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &Terminal) -> String {
        t.text()
    }

    #[test]
    fn lines_cr_bs_tab_and_erase() {
        let mut t = Terminal::default();
        t.feed(b"hello\r\nworld\rW\n");
        assert_eq!(text(&t), "hello\nWorld\n");
        t.feed(b"abc\x08\x08X\n");
        assert_eq!(t.lines()[2].iter().map(|c| c.c).collect::<String>(), "aXc");
        t.feed(b"a\tb\n");
        assert_eq!(t.lines()[3].iter().map(|c| c.c).collect::<String>(), "a       b");
        t.feed(b"progress 10%\r\x1b[Kprogress 99%\n");
        assert_eq!(t.lines()[4].iter().map(|c| c.c).collect::<String>(), "progress 99%");
        t.feed(b"x\x1b[3Cy\n");
        assert_eq!(t.lines()[5].iter().map(|c| c.c).collect::<String>(), "x   y");
        t.feed(b"\x1b[2J");
        assert_eq!(text(&t), "");
    }

    #[test]
    fn sgr_colours() {
        let mut t = Terminal::default();
        t.feed(b"\x1b[1;31mE\x1b[0mn\x1b[38;5;196mx\x1b[38;2;1;2;3my\x1b[42mz\x1b[mq");
        let l = &t.lines()[0];
        assert_eq!(l[0].style, Style { fg: Color::Idx(1), bold: true, ..Default::default() });
        assert_eq!(l[1].style, Style::default());
        assert_eq!(l[2].style.fg, Color::Idx(196));
        assert_eq!(l[3].style.fg, Color::Rgb(1, 2, 3));
        assert_eq!(l[4].style.bg, Color::Idx(2));
        assert_eq!(l[5].style, Style::default());
        assert_eq!(palette(196), (255, 0, 0));
        assert_eq!(palette(244), (128, 128, 128));
    }

    #[test]
    fn scrollback_is_bounded() {
        let mut t = Terminal::default();
        for i in 0..LINES_MAX + 10 {
            t.feed(format!("{i}\n").as_bytes());
        }
        assert_eq!(t.lines().len(), LINES_MAX);
        assert_eq!(t.lines()[0].iter().map(|c| c.c).collect::<String>(), "11");
    }
}
