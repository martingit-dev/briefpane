//! What the right panel shows: the agent's last answer and the repo's
//! uncommitted changes.

use std::io::{BufRead, BufReader};
use std::process::Command;

use serde::Serialize;
use serde_json::Value;

use crate::tmux::state_of;

/// Files past this size are listed but not diffed.
const MAX_FILE: u64 = 256 * 1024;
const MAX_FILES: usize = 30;

/// The agent's latest answer as markdown: Codex hands it over directly,
/// Claude's is the last assistant text in its transcript.
#[tauri::command]
pub fn brief(pane: String) -> Option<String> {
    let state = state_of(&pane)?;
    if !state.last.is_empty() {
        return Some(state.last);
    }
    if state.transcript.is_empty() {
        return None;
    }
    let file = std::fs::File::open(&state.transcript).ok()?;
    let mut last = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v["type"] != "assistant" {
            continue;
        }
        let text: Vec<&str> = v["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        if !text.is_empty() {
            last = Some(text.join("\n\n"));
        }
    }
    last
}

#[derive(Serialize)]
pub struct FileChange {
    name: String,
    /// `M`, `A`, `D` or `?` (untracked), from `git status --porcelain`.
    status: String,
    old: Option<String>,
    new: Option<String>,
}

fn git(dir: &str, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// Uncommitted changes in the repo containing `path`, old and new text per
/// file. Empty when `path` is not in a git repo.
#[tauri::command]
pub fn changes(path: String) -> Vec<FileChange> {
    let Some(root) = git(&path, &["rev-parse", "--show-toplevel"]) else { return vec![] };
    let root = String::from_utf8_lossy(&root).trim().to_string();
    let Some(status) = git(&root, &["status", "--porcelain", "-z", "--untracked-files=all"]) else { return vec![] };
    let text = |b: Vec<u8>| String::from_utf8(b).ok();
    // With -z a rename is two entries, `R  new` then `old`; drop the old name.
    let mut entries = Vec::new();
    let mut parts = status.split(|&b| b == 0).filter(|e| !e.is_empty());
    while let Some(e) = parts.next() {
        if e.first() == Some(&b'R') {
            parts.next();
        }
        if e.len() > 3 {
            entries.push(String::from_utf8_lossy(e).into_owned());
        }
    }
    entries
        .into_iter()
        .take(MAX_FILES)
        .map(|e| {
            let (code, name) = (e[..2].trim().to_string(), e[3..].to_string());
            let status = match code.as_str() {
                "??" => "?",
                c if c.contains('D') => "D",
                c if c.contains('A') => "A",
                _ => "M",
            }
            .to_string();
            let on_disk = std::path::Path::new(&root).join(&name);
            let small = on_disk.metadata().map(|m| m.len() <= MAX_FILE).unwrap_or(true);
            let old = (small && status != "?" && status != "A")
                .then(|| git(&root, &["show", &format!("HEAD:{name}")]).and_then(text))
                .flatten();
            let new = (small && status != "D").then(|| std::fs::read(&on_disk).ok().and_then(text)).flatten();
            FileChange { name, status, old, new }
        })
        .collect()
}

fn card_cli(path: &str, args: &[&str]) -> Result<String, String> {
    // A desktop launch may not have ~/.local/bin on PATH.
    let local = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/bin/briefpane-card");
    let bin = if local.exists() { local } else { "briefpane-card".into() };
    let out = Command::new(bin)
        .args(args)
        .args(["--cwd", path])
        .output()
        .map_err(|e| format!("briefpane-card: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The task card for the repo and branch at `path`, as `briefpane-card json`
/// prints it; `null` when there is none. One parser, shared with the agents.
#[tauri::command]
pub fn card(path: String) -> Result<Value, String> {
    serde_json::from_str(&card_cli(&path, &["json"])?).map_err(|e| e.to_string())
}

/// The human approves the card's plan, which lifts the edit gate.
#[tauri::command]
pub fn approve_plan(path: String) -> Result<(), String> {
    card_cli(&path, &["approve"]).map(|_| ())
}
