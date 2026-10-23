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
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
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
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// Uncommitted changes in the repo containing `path`, old and new text per
/// file. Empty when `path` is not in a git repo.
#[tauri::command]
pub fn changes(path: String) -> Vec<FileChange> {
    let Some(root) = git(&path, &["rev-parse", "--show-toplevel"]) else {
        return vec![];
    };
    let root = String::from_utf8_lossy(&root).trim().to_string();
    let Some(status) = git(
        &root,
        &["status", "--porcelain", "-z", "--untracked-files=all"],
    ) else {
        return vec![];
    };
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
            let small = on_disk
                .metadata()
                .map(|m| m.len() <= MAX_FILE)
                .unwrap_or(true);
            let old = (small && status != "?" && status != "A")
                .then(|| git(&root, &["show", &format!("HEAD:{name}")]).and_then(text))
                .flatten();
            let new = (small && status != "D")
                .then(|| std::fs::read(&on_disk).ok().and_then(text))
                .flatten();
            FileChange {
                name,
                status,
                old,
                new,
            }
        })
        .collect()
}

fn card_cli(path: &str, args: &[&str]) -> Result<String, String> {
    // A desktop launch may not have ~/.local/bin on PATH.
    let local = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".local/bin/briefpane-card");
    let bin = if local.exists() {
        local
    } else {
        "briefpane-card".into()
    };
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

const REVIEW_PROMPT: &str = "Review this change with fresh eyes. Read the task card with `briefpane-card show`, then the change with `git status`, `git diff` and `git diff --staged`. Judge three things: does it do what Define says, is every Proof real evidence, and are there bugs, missed cases or needless code. Do not edit any file. Record each finding with `briefpane-card finding <P0|P1|P2> \"<what is wrong, and where>\"`: P0 breaks the Define, P1 is a real bug or gap, P2 is a cleanup. When done, run `briefpane-card next \"review done\"` and stop.";

/// Opens a fresh session of the same agent in a new window of `session`,
/// reviewing the repo at `path` against its card. Claude runs with edits
/// disallowed; Codex is told not to edit. Returns the new pane id.
#[tauri::command]
pub fn start_review(
    session: String,
    path: String,
    agent: String,
    builder: String,
) -> Result<String, String> {
    let mut cmd: Vec<&str> = vec![
        "tmux",
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-n",
        "review",
        "-c",
        &path,
    ];
    let target = format!("{session}:");
    cmd.extend(["-t", &target]);
    // BRIEFPANE_CLAUDE / BRIEFPANE_CODEX point at a different binary.
    let bin = std::env::var(format!("BRIEFPANE_{}", agent.to_uppercase()))
        .unwrap_or_else(|_| agent.clone());
    match agent.as_str() {
        "claude" => cmd.extend([
            bin.as_str(),
            "--disallowedTools",
            "Edit,Write,MultiEdit,NotebookEdit",
            REVIEW_PROMPT,
        ]),
        "codex" => cmd.extend([bin.as_str(), REVIEW_PROMPT]),
        other => return Err(format!("no reviewer for {other}")),
    }
    let out = Command::new(cmd[0])
        .args(&cmd[1..])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    card_cli(&path, &["phase", "review"])?;
    card_cli(&path, &["builder", &builder])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The human's verdict on one review finding.
#[tauri::command]
pub fn dispose(path: String, id: String, verdict: String) -> Result<(), String> {
    card_cli(&path, &["dispose", &id, &verdict]).map(|_| ())
}
