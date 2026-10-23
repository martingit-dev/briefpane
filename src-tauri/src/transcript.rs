//! The agent's latest answer. Codex hands it to the hook directly; Claude's
//! is the last assistant text in its JSONL transcript, which only grows, so
//! each call parses just the lines appended since the last one.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::{Value, json};

use crate::{log, state};

/// The last answer in one transcript, up to `offset`: the end of its last
/// complete line when it was read.
struct Parsed {
    len: u64,
    mtime: SystemTime,
    offset: u64,
    last: Option<String>,
}

#[derive(Default)]
pub struct Transcripts(Mutex<HashMap<PathBuf, Parsed>>);

/// The text of an assistant message, its text blocks joined; `None` for any
/// other line or an assistant line without text (a tool call).
pub fn answer_of(line: &Value) -> Option<String> {
    if line["type"] != "assistant" {
        return None;
    }
    let text: Vec<&str> = line["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    (!text.is_empty()).then(|| text.join("\n\n"))
}

/// The last answer in a run of complete JSONL lines, and how many lines
/// were not JSON.
pub fn last_answer(lines: &[u8]) -> (Option<String>, usize) {
    let mut last = None;
    let mut bad = 0;
    for line in lines.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
        match serde_json::from_slice::<Value>(line) {
            Ok(v) => {
                if let Some(a) = answer_of(&v) {
                    last = Some(a);
                }
            }
            Err(_) => bad += 1,
        }
    }
    (last, bad)
}

const CHUNK: u64 = 1 << 20;

/// Reads `file` backwards from `end`, a chunk at a time, prepending each to
/// `buf`, which holds the bytes from the returned position on.
fn read_before(file: &mut File, pos: u64, buf: &mut Vec<u8>) -> std::io::Result<u64> {
    let start = pos.saturating_sub(CHUNK);
    let mut chunk = vec![0; (pos - start) as usize];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut chunk)?;
    chunk.append(buf);
    *buf = chunk;
    Ok(start)
}

/// The last answer in a whole transcript, found from its end, so a first
/// read costs the last answer's distance from the end, not the file's size.
/// Returns the answer, the lines that were not JSON, and the end of the
/// last complete line.
fn last_answer_backward(
    file: &mut File,
    len: u64,
) -> std::io::Result<(Option<String>, usize, u64)> {
    let mut pos = len;
    let mut buf = Vec::new();
    // `buf` ends just before a newline from here on.
    let complete = loop {
        if let Some(i) = buf.iter().rposition(|&b| b == b'\n') {
            buf.truncate(i);
            break pos + i as u64 + 1;
        }
        if pos == 0 {
            return Ok((None, 0, 0));
        }
        pos = read_before(file, pos, &mut buf)?;
    };
    let mut bad = 0;
    loop {
        let cut = buf.iter().rposition(|&b| b == b'\n');
        if cut.is_none() && pos > 0 {
            pos = read_before(file, pos, &mut buf)?;
            continue;
        }
        let line = &buf[cut.map_or(0, |i| i + 1)..];
        if !line.is_empty() {
            match serde_json::from_slice::<Value>(line) {
                Ok(v) => {
                    if let Some(a) = answer_of(&v) {
                        return Ok((Some(a), bad, complete));
                    }
                }
                Err(_) => bad += 1,
            }
        }
        match cut {
            Some(i) => buf.truncate(i),
            None => return Ok((None, bad, complete)),
        }
    }
}

impl Transcripts {
    fn last(&self, path: PathBuf) -> std::io::Result<Option<String>> {
        let meta = std::fs::metadata(&path)?;
        let (len, mtime) = (meta.len(), meta.modified()?);
        let mut cache = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = cache.get(&path)
            && p.len == len
            && p.mtime == mtime
        {
            return Ok(p.last.clone());
        }
        let prior = cache.remove(&path);
        // A file that shrank was rewritten: start over.
        let (from, mut last) = match prior {
            Some(p) if p.offset <= len => (p.offset, p.last),
            _ => (0, None),
        };
        let mut file = File::open(&path)?;
        // A line still being written stays unread until its newline lands.
        let (found, bad, complete) = if from == 0 {
            last_answer_backward(&mut file, len)?
        } else {
            let mut tail = Vec::new();
            file.seek(SeekFrom::Start(from))?;
            file.take(len - from).read_to_end(&mut tail)?;
            let complete = tail.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
            let (found, bad) = last_answer(&tail[..complete]);
            (found, bad, from + complete as u64)
        };
        if bad > 0 {
            log::event(
                "error",
                json!({ "msg": "transcript lines not JSON", "path": path, "count": bad }),
            );
        }
        if found.is_some() {
            last = found;
        }
        cache.insert(
            path,
            Parsed {
                len,
                mtime,
                offset: complete,
                last: last.clone(),
            },
        );
        Ok(last)
    }
}

pub fn brief(cache: &Transcripts, pane: &str) -> Option<String> {
    let state = state::state_of(pane)?;
    if !state.last.is_empty() {
        return Some(state.last);
    }
    if state.transcript.is_empty() {
        return None;
    }
    cache
        .last(PathBuf::from(&state.transcript))
        .unwrap_or_else(|e| {
            log::error("transcript unreadable", e);
            None
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_assistant_text_wins() {
        let lines = br#"{"type":"user","message":{"content":"hi"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"first"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"a"},{"type":"thinking"},{"type":"text","text":"b"}]}}
{"type":"user","message":{"content":[{"type":"tool_result"}]}}
"#;
        assert_eq!(last_answer(lines), (Some("a\n\nb".into()), 0));
    }

    #[test]
    fn tool_calls_keep_the_earlier_answer() {
        let lines = br#"{"type":"assistant","message":{"content":[{"type":"text","text":"done"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use"}]}}
"#;
        assert_eq!(last_answer(lines).0.as_deref(), Some("done"));
    }

    #[test]
    fn bad_lines_are_counted() {
        let lines = b"not json\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"x\"}]}}\n{\"trunc";
        assert_eq!(last_answer(lines), (Some("x".into()), 2));
        assert_eq!(last_answer(b""), (None, 0));
    }

    #[test]
    fn backward_matches_forward() {
        let dir = std::env::temp_dir().join(format!("bp-backward-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let answer = |t: &str| {
            format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{t}\"}}]}}}}\n"
            )
        };
        let big = format!("{{\"type\":\"user\",\"x\":\"{}\"}}\n", "y".repeat(3 << 20));
        let cases = [
            String::new(),
            "no newline yet".to_string(),
            format!("{}{}", answer("a"), big),
            format!(
                "{}{}{}junk\n\n{}",
                answer("a"),
                big,
                answer("b"),
                &big[..100]
            ),
            format!("{big}{big}"),
        ];
        for text in cases {
            std::fs::write(&path, &text).unwrap();
            let mut f = File::open(&path).unwrap();
            let (got, _, end) = last_answer_backward(&mut f, text.len() as u64).unwrap();
            let complete = text.rfind('\n').map_or(0, |i| i + 1);
            assert_eq!(got, last_answer(&text.as_bytes()[..complete]).0);
            assert_eq!(end, complete as u64);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn appended_tail_is_parsed() {
        let dir = std::env::temp_dir().join(format!("bp-transcript-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let line = |t: &str| {
            format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{t}\"}}]}}}}\n"
            )
        };
        std::fs::write(&path, line("one")).unwrap();
        let cache = Transcripts::default();
        assert_eq!(cache.last(path.clone()).unwrap().as_deref(), Some("one"));
        // Half a line: not read yet.
        let mut text = line("one");
        let two = line("two");
        text.push_str(&two[..10]);
        std::fs::write(&path, &text).unwrap();
        assert_eq!(cache.last(path.clone()).unwrap().as_deref(), Some("one"));
        text.push_str(&two[10..]);
        text.push_str(r#"{"type":"user"}"#);
        text.push('\n');
        std::fs::write(&path, &text).unwrap();
        assert_eq!(cache.last(path.clone()).unwrap().as_deref(), Some("two"));
        // Rewritten shorter: parsed from the start.
        std::fs::write(&path, line("new")).unwrap();
        assert_eq!(cache.last(path.clone()).unwrap().as_deref(), Some("new"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
