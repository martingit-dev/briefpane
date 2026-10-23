//! Parser for tmux control mode (`tmux -C`). Each line tmux writes is one
//! notification; replies to our own commands arrive between `%begin` and
//! `%end` and are surfaced as `Reply` lines so the caller can ignore them.
//! Protocol: https://github.com/tmux/tmux/wiki/Control-Mode

#[derive(Debug, PartialEq)]
pub enum Event {
    /// Bytes a pane wrote to its terminal, already unescaped.
    Output {
        pane: String,
        data: Vec<u8>,
    },
    WindowAdd {
        window: String,
    },
    WindowClose {
        window: String,
    },
    WindowRenamed {
        window: String,
        name: String,
    },
    SessionChanged {
        session: String,
        name: String,
    },
    /// A line inside a `%begin`/`%end` block.
    Reply(String),
    /// tmux detached this client; carries the reason when given.
    Exit(Option<String>),
    /// Any notification this crate does not model yet.
    Other(String),
}

/// Tracks whether the next lines belong to a command reply block.
#[derive(Default)]
pub struct Parser {
    in_block: bool,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses one line without its trailing newline. Returns `None` for the
    /// `%begin`/`%end`/`%error` markers themselves.
    pub fn line(&mut self, raw: &[u8]) -> Option<Event> {
        if self.in_block {
            if raw.starts_with(b"%end ") || raw.starts_with(b"%error ") {
                self.in_block = false;
                return None;
            }
            return Some(Event::Reply(String::from_utf8_lossy(raw).into_owned()));
        }
        if raw.starts_with(b"%begin ") {
            self.in_block = true;
            return None;
        }
        if let Some(rest) = raw.strip_prefix(b"%output ") {
            let (pane, data) = split_word(rest);
            return Some(Event::Output {
                pane: text(pane),
                data: unescape(data),
            });
        }
        let s = String::from_utf8_lossy(raw);
        let mut words = s.splitn(3, ' ');
        let kind = words.next().unwrap_or("");
        let a = words.next().unwrap_or("").to_string();
        let b = words.next().unwrap_or("").to_string();
        Some(match kind {
            "%window-add" => Event::WindowAdd { window: a },
            "%window-close" | "%unlinked-window-close" => Event::WindowClose { window: a },
            "%window-renamed" => Event::WindowRenamed { window: a, name: b },
            "%session-changed" => Event::SessionChanged {
                session: a,
                name: b,
            },
            "%exit" => Event::Exit(if a.is_empty() {
                None
            } else {
                Some(format!("{a} {b}").trim().to_string())
            }),
            _ => Event::Other(s.into_owned()),
        })
    }
}

fn split_word(s: &[u8]) -> (&[u8], &[u8]) {
    match s.iter().position(|&c| c == b' ') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, &[]),
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// tmux writes bytes below 0x20 and the backslash as `\ooo` octal.
pub fn unescape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\'
            && i + 3 < s.len()
            && s[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            out.push((s[i + 1] - b'0') * 64 + (s[i + 2] - b'0') * 8 + (s[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// Formats bytes for `send-keys -H`, which takes each byte as a hex word.
pub fn send_keys_hex(pane: &str, data: &[u8]) -> String {
    use std::fmt::Write;
    let mut line = String::with_capacity(20 + data.len() * 3);
    let _ = write!(line, "send-keys -t {pane} -H");
    for b in data {
        let _ = write!(line, " {b:02x}");
    }
    line.push('\n');
    line
}

/// Collects pane output between flushes so each pane gets one frame per
/// read burst instead of one per `%output` line.
#[derive(Default)]
pub struct Coalescer {
    frames: Vec<(String, Vec<u8>)>,
    bytes: usize,
}

impl Coalescer {
    pub fn push(&mut self, pane: String, data: Vec<u8>) {
        self.bytes += data.len();
        match self.frames.iter_mut().find(|(p, _)| *p == pane) {
            Some((_, buf)) => buf.extend_from_slice(&data),
            None => self.frames.push((pane, data)),
        }
    }

    /// Bytes held since the last flush.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// One frame per pane, in the order each pane first wrote.
    pub fn take(&mut self) -> Vec<(String, Vec<u8>)> {
        self.bytes = 0;
        std::mem::take(&mut self.frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_unescaped() {
        let mut p = Parser::new();
        let e = p.line(br"%output %3 hi\015\012a\134b").unwrap();
        assert_eq!(
            e,
            Event::Output {
                pane: "%3".into(),
                data: b"hi\r\na\\b".to_vec()
            }
        );
    }

    #[test]
    fn utf8_passes_through() {
        let mut p = Parser::new();
        let e = p.line("%output %1 ● ok".as_bytes()).unwrap();
        assert_eq!(
            e,
            Event::Output {
                pane: "%1".into(),
                data: "● ok".as_bytes().to_vec()
            }
        );
    }

    #[test]
    fn reply_block_from_real_tmux() {
        let mut p = Parser::new();
        assert_eq!(p.line(b"%begin 1792731501 650959 1"), None);
        assert_eq!(
            p.line(b"0: api (1 panes) [211x56] [layout 31a6,211x56,0,0,173] @135"),
            Some(Event::Reply(
                "0: api (1 panes) [211x56] [layout 31a6,211x56,0,0,173] @135".into()
            ))
        );
        // a line that looks like a notification is still part of the reply
        assert!(matches!(p.line(b"%output %9 x"), Some(Event::Reply(_))));
        assert_eq!(p.line(b"%end 1792731501 650959 1"), None);
        assert_eq!(
            p.line(b"%session-changed $48 heya"),
            Some(Event::SessionChanged {
                session: "$48".into(),
                name: "heya".into()
            })
        );
        assert_eq!(p.line(b"%exit"), Some(Event::Exit(None)));
    }

    #[test]
    fn trailing_backslash_is_kept() {
        assert_eq!(unescape(br"a\"), b"a\\".to_vec());
        assert_eq!(unescape(br"\01"), b"\\01".to_vec());
    }

    #[test]
    fn hex_keys() {
        assert_eq!(
            send_keys_hex("%3", b"ls\r"),
            "send-keys -t %3 -H 6c 73 0d\n"
        );
        assert_eq!(
            send_keys_hex("%3", &[0x00, 0xff]),
            "send-keys -t %3 -H 00 ff\n"
        );
    }

    #[test]
    fn coalesces_per_pane() {
        let mut c = Coalescer::default();
        c.push("%1".into(), b"ab".to_vec());
        c.push("%2".into(), b"x".to_vec());
        c.push("%1".into(), b"c".to_vec());
        assert_eq!(c.bytes(), 4);
        assert_eq!(
            c.take(),
            vec![("%1".into(), b"abc".to_vec()), ("%2".into(), b"x".to_vec())]
        );
        assert_eq!(c.bytes(), 0);
        assert!(c.take().is_empty());
    }
}
