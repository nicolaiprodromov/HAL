//! Minimal ANSI parser for the Output pane: SGR becomes ratatui styles,
//! every other escape sequence is dropped.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// One per stream. SGR state persists across lines until reset.
pub struct AnsiParser {
    base: Style,
    cur: Style,
    /// Unfinished escape sequence at the end of the previous chunk.
    pending: String,
}

impl AnsiParser {
    pub fn new(base: Style) -> Self {
        Self {
            base,
            cur: base,
            pending: String::new(),
        }
    }

    /// Parse one line (no trailing `\n`). Text before the last `\r` is
    /// discarded, approximating a carriage-return overwrite.
    pub fn line(&mut self, text: &str) -> Line<'static> {
        let text = text.strip_suffix('\r').unwrap_or(text);
        let input = if self.pending.is_empty() {
            text.to_string()
        } else {
            let mut s = std::mem::take(&mut self.pending);
            s.push_str(text);
            s
        };
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut buf = String::new();
        let mut it = input.char_indices().peekable();
        while let Some((i, c)) = it.next() {
            match c {
                '\r' => {
                    buf.clear();
                    spans.clear();
                }
                '\x1b' => {
                    let rest = &input[i + 1..];
                    match skip_escape(rest) {
                        Esc::Incomplete => {
                            self.pending = input[i..].to_string();
                            break;
                        }
                        Esc::Done { len, sgr } => {
                            if let Some(params) = sgr {
                                if !buf.is_empty() {
                                    spans.push(Span::styled(std::mem::take(&mut buf), self.cur));
                                }
                                self.apply_sgr(params);
                            }
                            // `len` counts bytes after ESC.
                            while it.peek().is_some_and(|&(j, _)| j < i + 1 + len) {
                                it.next();
                            }
                        }
                    }
                }
                c if c.is_control() && c != '\t' => {}
                c => buf.push(c),
            }
        }
        if !buf.is_empty() {
            spans.push(Span::styled(buf, self.cur));
        }
        Line::from(spans)
    }

    fn apply_sgr(&mut self, params: &str) {
        let nums: Vec<u32> = if params.is_empty() {
            vec![0]
        } else {
            params
                .split([';', ':'])
                .map(|p| p.parse().unwrap_or(0))
                .collect()
        };
        let mut i = 0;
        while i < nums.len() {
            let n = nums[i];
            i += 1;
            let s = &mut self.cur;
            match n {
                0 => *s = self.base,
                1 => *s = s.add_modifier(Modifier::BOLD),
                2 => *s = s.add_modifier(Modifier::DIM),
                3 => *s = s.add_modifier(Modifier::ITALIC),
                4 => *s = s.add_modifier(Modifier::UNDERLINED),
                7 => *s = s.add_modifier(Modifier::REVERSED),
                22 => *s = s.remove_modifier(Modifier::BOLD | Modifier::DIM),
                23 => *s = s.remove_modifier(Modifier::ITALIC),
                24 => *s = s.remove_modifier(Modifier::UNDERLINED),
                27 => *s = s.remove_modifier(Modifier::REVERSED),
                30..=37 => *s = s.fg(basic(n - 30)),
                40..=47 => *s = s.bg(basic(n - 40)),
                90..=97 => *s = s.fg(bright(n - 90)),
                100..=107 => *s = s.bg(bright(n - 100)),
                39 => s.fg = self.base.fg,
                49 => s.bg = self.base.bg,
                38 | 48 => {
                    let color = match nums.get(i) {
                        Some(5) => {
                            let c = nums.get(i + 1).map(|&v| Color::Indexed(v.min(255) as u8));
                            i += 2;
                            c
                        }
                        Some(2) => {
                            let c = match nums.get(i + 1..i + 4) {
                                Some(&[r, g, b]) => Some(Color::Rgb(
                                    r.min(255) as u8,
                                    g.min(255) as u8,
                                    b.min(255) as u8,
                                )),
                                _ => None,
                            };
                            i += 4;
                            c
                        }
                        _ => {
                            i = nums.len();
                            None
                        }
                    };
                    if let Some(c) = color {
                        *s = if n == 38 { s.fg(c) } else { s.bg(c) };
                    }
                }
                _ => {}
            }
        }
    }
}

fn basic(n: u32) -> Color {
    [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
    ][n as usize & 7]
}

fn bright(n: u32) -> Color {
    [
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ][n as usize & 7]
}

enum Esc<'a> {
    Incomplete,
    /// `len` bytes after ESC belong to the sequence; `sgr` holds the
    /// parameters when it is an SGR (`CSI ... m`).
    Done {
        len: usize,
        sgr: Option<&'a str>,
    },
}

/// Classify the sequence that follows an ESC byte.
fn skip_escape(rest: &str) -> Esc<'_> {
    let b = rest.as_bytes();
    let Some(&first) = b.first() else {
        return Esc::Incomplete;
    };
    match first {
        b'[' => {
            // CSI: params 0x30-0x3F, intermediates 0x20-0x2F, final 0x40-0x7E.
            for (k, &x) in b.iter().enumerate().skip(1) {
                if (0x40..=0x7e).contains(&x) {
                    let sgr = (x == b'm'
                        && b[1..k]
                            .iter()
                            .all(|c| c.is_ascii_digit() || *c == b';' || *c == b':'))
                    .then(|| &rest[1..k]);
                    return Esc::Done { len: k + 1, sgr };
                }
                if !(0x20..=0x3f).contains(&x) {
                    return Esc::Done { len: k, sgr: None };
                }
            }
            Esc::Incomplete
        }
        b']' | b'P' | b'_' | b'^' | b'X' => {
            // String sequences end at BEL or ST (ESC \).
            let mut k = 1;
            while k < b.len() {
                if b[k] == 0x07 {
                    return Esc::Done {
                        len: k + 1,
                        sgr: None,
                    };
                }
                if b[k] == 0x1b {
                    return match b.get(k + 1) {
                        Some(b'\\') => Esc::Done {
                            len: k + 2,
                            sgr: None,
                        },
                        Some(_) => Esc::Done { len: k, sgr: None },
                        None => Esc::Incomplete,
                    };
                }
                k += 1;
            }
            Esc::Incomplete
        }
        // Charset selection and similar take one more byte.
        b'(' | b')' | b'*' | b'+' | b'#' | b'%' => {
            if b.len() < 2 {
                Esc::Incomplete
            } else {
                Esc::Done { len: 2, sgr: None }
            }
        }
        _ => Esc::Done {
            len: rest.chars().next().map_or(1, char::len_utf8),
            sgr: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> AnsiParser {
        AnsiParser::new(Style::default().fg(Color::White))
    }

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn plain_text_uses_base_style() {
        let l = p().line("hello");
        assert_eq!(text(&l), "hello");
        assert_eq!(l.spans[0].style.fg, Some(Color::White));
    }

    #[test]
    fn basic_and_bright_colors() {
        let l = p().line("\x1b[31ma\x1b[0mb\x1b[92mc\x1b[44md");
        assert_eq!(text(&l), "abcd");
        assert_eq!(l.spans[0].style.fg, Some(Color::Red));
        assert_eq!(l.spans[1].style.fg, Some(Color::White));
        assert_eq!(l.spans[2].style.fg, Some(Color::LightGreen));
        assert_eq!(l.spans[3].style.bg, Some(Color::Blue));
    }

    #[test]
    fn indexed_and_truecolor() {
        let l = p().line("\x1b[38;5;200ma\x1b[48;2;1;2;3mb");
        assert_eq!(l.spans[0].style.fg, Some(Color::Indexed(200)));
        assert_eq!(l.spans[1].style.bg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(l.spans[1].style.fg, Some(Color::Indexed(200)));
    }

    #[test]
    fn modifiers_and_reset() {
        let l = p().line("\x1b[1;3;4;7mx\x1b[22;23;24;27my\x1b[2mz\x1b[mw");
        let m = l.spans[0].style.add_modifier;
        assert!(m.contains(
            Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED | Modifier::REVERSED
        ));
        assert!(l.spans[1].style.add_modifier.is_empty());
        assert!(l.spans[2].style.add_modifier.contains(Modifier::DIM));
        assert!(l.spans[3].style.add_modifier.is_empty());
    }

    #[test]
    fn state_carries_across_lines() {
        let mut parser = p();
        parser.line("\x1b[32mone");
        let l = parser.line("two");
        assert_eq!(l.spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn stderr_default_fg_survives_reset() {
        let mut parser = AnsiParser::new(Style::default().fg(Color::Red));
        let l = parser.line("\x1b[34ma\x1b[0mb\x1b[39mc");
        assert_eq!(l.spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn non_sgr_sequences_are_stripped() {
        let l = p().line(
            "\x1b[2J\x1b[H\x1b[?1049h\x1b[?25lA\x1b]0;title\x07B\x1b]8;;u\x1b\\C\x1b(0D\x1b7E",
        );
        assert_eq!(text(&l), "ABCDE");
    }

    #[test]
    fn carriage_return_keeps_last_segment() {
        assert_eq!(text(&p().line("abc\rxyz")), "xyz");
        assert_eq!(text(&p().line("abc\r")), "abc");
    }

    #[test]
    fn escape_split_across_chunks() {
        let mut parser = p();
        let a = parser.line("x\x1b[3");
        assert_eq!(text(&a), "x");
        let b = parser.line("1my");
        assert_eq!(text(&b), "y");
        assert_eq!(b.spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn multibyte_text_after_escape() {
        let l = p().line("\x1b[1m█▀\x1b[0m é");
        assert_eq!(text(&l), "█▀ é");
    }
}
