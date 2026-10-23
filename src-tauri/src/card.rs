//! The task card through `briefpane-card`, and the review window. Reading a
//! card costs no process while its file is unchanged: the card's path is
//! asked once per directory and branch, and `json` is rerun only when the
//! file's size or mtime moves.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde_json::Value;

use crate::proc;
use crate::tmux::{pane_id_ok, tmux};

fn card_cli(path: &str, args: &[&str]) -> Result<String, String> {
    // A desktop launch may not have ~/.local/bin on PATH.
    let local = crate::home().join(".local/bin/briefpane-card");
    let bin = if local.exists() {
        local.to_string_lossy().into_owned()
    } else {
        "briefpane-card".into()
    };
    let mut all = args.to_vec();
    all.extend(["--cwd", path]);
    Ok(proc::text(proc::run(&bin, &all)?))
}

/// What `HEAD` holds for the repo containing `dir`, found without running
/// git: it names the branch, and the card's path follows the branch.
/// `None` outside a repo, where no card can exist.
fn head_of(dir: &Path) -> Option<String> {
    let dot = dir
        .ancestors()
        .map(|d| d.join(".git"))
        .find(|g| g.exists())?;
    let git_dir = if dot.is_file() {
        // A worktree: `.git` is a file naming the real git dir.
        let link = std::fs::read_to_string(&dot).ok()?;
        let target = PathBuf::from(link.strip_prefix("gitdir:")?.trim());
        dot.parent()?.join(target)
    } else {
        dot
    };
    std::fs::read_to_string(git_dir.join("HEAD")).ok()
}

type Stamp = Option<(u64, SystemTime)>;

fn stamp(path: &Path) -> Stamp {
    let m = path.metadata().ok()?;
    Some((m.len(), m.modified().ok()?))
}

#[derive(Default)]
pub struct Cards {
    /// Card path by (directory, its repo's HEAD).
    paths: Mutex<HashMap<(String, String), PathBuf>>,
    /// Parsed card by its path, with the file's stamp when read.
    cards: Mutex<HashMap<PathBuf, (Stamp, Value)>>,
}

impl Cards {
    /// The task card for the repo and branch at `path`, as
    /// `briefpane-card json` prints it; `null` when there is none.
    pub fn card(&self, path: &str) -> Result<Value, String> {
        let Some(head) = head_of(Path::new(path)) else {
            return Err(format!("{path} is not in a git repo"));
        };
        let key = (path.to_string(), head);
        let known = self
            .paths
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned();
        let file = match known {
            Some(f) => f,
            None => {
                let f = PathBuf::from(card_cli(path, &["path"])?.trim());
                self.paths
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key, f.clone());
                f
            }
        };
        let now = stamp(&file);
        if now.is_none() {
            return Ok(Value::Null);
        }
        if let Some((s, v)) = self
            .cards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&file)
            && *s == now
        {
            return Ok(v.clone());
        }
        let v: Value =
            serde_json::from_str(&card_cli(path, &["json"])?).map_err(|e| e.to_string())?;
        self.cards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(file, (now, v.clone()));
        Ok(v)
    }
}

/// The human approves the card's plan, which lifts the edit gate.
pub fn approve_plan(path: &str) -> Result<(), String> {
    card_cli(path, &["approve"]).map(|_| ())
}

/// The human's verdict on one review finding.
pub fn dispose(path: &str, id: &str, verdict: &str) -> Result<(), String> {
    card_cli(path, &["dispose", id, verdict]).map(|_| ())
}

const REVIEW_PROMPT: &str = "Review this change with fresh eyes. Read the task card with `briefpane-card show`, then the change with `git status`, `git diff` and `git diff --staged`. Judge three things: does it do what Define says, is every Proof real evidence, and are there bugs, missed cases or needless code. Do not edit any file. Record each finding with `briefpane-card finding <P0|P1|P2> \"<what is wrong, and where>\"`: P0 breaks the Define, P1 is a real bug or gap, P2 is a cleanup. When done, run `briefpane-card next \"review done\"` and stop.";

/// Opens a fresh session of the same agent in a new window of `session`,
/// reviewing the repo at `path` against its card. Claude runs with edits
/// disallowed; Codex is told not to edit. Returns the new pane id.
pub fn start_review(
    session: &str,
    path: &str,
    agent: &str,
    builder: &str,
) -> Result<String, String> {
    pane_id_ok(builder)?;
    let target = format!("{session}:");
    // BRIEFPANE_CLAUDE / BRIEFPANE_CODEX point at a different binary.
    let bin = std::env::var(format!("BRIEFPANE_{}", agent.to_uppercase()))
        .unwrap_or_else(|_| agent.to_string());
    let mut args = vec![
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-n",
        "review",
        "-c",
        path,
        "-t",
        &target,
    ];
    match agent {
        "claude" => args.extend([
            bin.as_str(),
            "--disallowedTools",
            "Edit,Write,MultiEdit,NotebookEdit",
            REVIEW_PROMPT,
        ]),
        "codex" => args.extend([bin.as_str(), REVIEW_PROMPT]),
        other => return Err(format!("no reviewer for {other}")),
    }
    let pane = tmux(&args)?;
    card_cli(path, &["phase", "review"])?;
    card_cli(path, &["builder", builder])?;
    Ok(pane.trim().to_string())
}
