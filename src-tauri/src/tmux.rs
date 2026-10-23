//! Bridges tmux to the window. One control-mode client per tmux session
//! streams pane output as events; listing and capture use plain `tmux`
//! calls. The app is one more tmux client: with tmux's default
//! `window-size latest`, whichever client you used last sets a window's size.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tmux_cc::{Event, Parser, send_keys_hex};

#[derive(Default)]
pub struct Tmux {
    clients: Mutex<HashMap<String, ChildStdin>>,
}

#[derive(Serialize)]
pub struct Pane {
    session: String,
    window_id: String,
    window_index: u32,
    window_name: String,
    pane_id: String,
    command: String,
    path: String,
    width: u16,
    height: u16,
    /// The title the program set; Claude and Codex put the task name here.
    title: String,
    /// `claude` or `codex` when one runs in the pane, judged by process.
    agent: Option<String>,
    kind: Kind,
    state: Option<AgentState>,
}

#[derive(Serialize, Clone)]
struct Output {
    pane: String,
    data: Vec<u8>,
}

#[derive(Serialize)]
pub struct Snapshot {
    /// Scrollback above the screen, oldest first.
    history: String,
    /// The visible screen, one line per row.
    text: String,
    cursor_x: u16,
    cursor_y: u16,
    /// The pane's size when captured, so the view can wait out a resize.
    width: u16,
    height: u16,
}

fn tmux(args: &[&str]) -> Result<String, String> {
    let out = Command::new("tmux").args(args).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

const PANE_FORMAT: &str = "#{session_name}\t#{window_id}\t#{window_index}\t#{window_name}\t#{pane_id}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_width}\t#{pane_height}\t#{pane_title}\t#{pane_pid}";

const AGENTS: [&str; 2] = ["claude", "codex"];

const SHELLS: [&str; 5] = ["bash", "zsh", "fish", "sh", "dash"];

/// What a pane is for, judged by its processes: an agent you talk to, a
/// service you watch, or a bare shell.
#[derive(Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Agent,
    Service,
    Terminal,
}

fn children(pid: &str) -> Vec<String> {
    std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// Finds an agent in the pane's shell or its direct children. The command
/// name alone misleads: Codex runs as `node`.
fn agent_of(pid: &str, kids: &[String]) -> Option<String> {
    std::iter::once(pid)
        .chain(kids.iter().map(String::as_str))
        .flat_map(|p| {
            let raw = std::fs::read(format!("/proc/{p}/cmdline")).unwrap_or_default();
            // argv[0] and argv[1]: `claude`, or `node /path/to/codex`.
            raw.split(|&b| b == 0).take(2).map(|a| String::from_utf8_lossy(a).into_owned()).collect::<Vec<_>>()
        })
        .find_map(|arg| {
            let base = arg.rsplit('/').next().unwrap_or("").to_string();
            AGENTS.iter().find(|a| base == **a).map(|a| a.to_string())
        })
}

/// What `briefpane-hook` last recorded for a pane.
#[derive(Serialize, Deserialize, Clone)]
pub struct AgentState {
    state: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    pub transcript: String,
    #[serde(default)]
    pub last: String,
    at: u64,
}

pub fn state_of(pane_id: &str) -> Option<AgentState> {
    let dir = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state"));
    let raw = std::fs::read(dir.join("briefpane/panes").join(format!("{}.json", pane_id.trim_start_matches('%')))).ok()?;
    serde_json::from_slice(&raw).ok()
}

#[tauri::command]
pub fn list_panes() -> Result<Vec<Pane>, String> {
    let out = tmux(&["list-panes", "-a", "-F", PANE_FORMAT])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 11 {
                return None;
            }
            let kids = children(f[10]);
            let agent = agent_of(f[10], &kids);
            let kind = match &agent {
                Some(_) => Kind::Agent,
                None if kids.is_empty() && SHELLS.contains(&f[5]) => Kind::Terminal,
                None => Kind::Service,
            };
            let state = agent.as_ref().and_then(|_| state_of(f[4]));
            Some(Pane {
                session: f[0].into(),
                window_id: f[1].into(),
                window_index: f[2].parse().unwrap_or(0),
                window_name: f[3].into(),
                pane_id: f[4].into(),
                command: f[5].into(),
                path: f[6].into(),
                width: f[7].parse().unwrap_or(80),
                height: f[8].parse().unwrap_or(24),
                title: f[9].into(),
                agent,
                kind,
                state,
            })
        })
        .collect())
}

/// What the pane shows now, with colours, so the view starts filled before
/// live output arrives.
#[tauri::command]
pub fn capture(pane: String) -> Result<Snapshot, String> {
    let history = tmux(&["capture-pane", "-p", "-e", "-S", "-5000", "-E", "-1", "-t", &pane])?;
    let text = tmux(&["capture-pane", "-p", "-e", "-t", &pane])?;
    let cur = tmux(&["display-message", "-p", "-t", &pane, "#{cursor_x} #{cursor_y} #{pane_width} #{pane_height}"])?;
    let mut it = cur.split_whitespace().map(|n| n.parse().unwrap_or(0));
    let mut next = || it.next().unwrap_or(0);
    Ok(Snapshot { history, text, cursor_x: next(), cursor_y: next(), width: next(), height: next() })
}

/// Starts streaming a session's panes. Idempotent per session.
#[tauri::command]
pub fn attach(app: AppHandle, state: State<Tmux>, session: String) -> Result<(), String> {
    let mut clients = state.clients.lock().unwrap();
    if clients.contains_key(&session) {
        return Ok(());
    }
    let mut child = Command::new("tmux")
        .args(["-C", "attach-session", "-t", &session])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("no stdout")?;
    clients.insert(session.clone(), child.stdin.take().ok_or("no stdin")?);

    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut parser = Parser::new();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if buf.last() == Some(&b'\n') {
                buf.pop();
            }
            match parser.line(&buf) {
                Some(Event::Output { pane, data }) => {
                    let _ = app.emit("pane-output", Output { pane, data });
                }
                Some(Event::Exit(_)) => break,
                Some(Event::WindowAdd { .. } | Event::WindowClose { .. } | Event::WindowRenamed { .. }) => {
                    let _ = app.emit("panes-changed", ());
                }
                // Pane sizes changed: a split, or another client took over the size.
                Some(Event::Other(line)) if line.starts_with("%layout-change ") => {
                    let _ = app.emit("panes-changed", ());
                }
                _ => {}
            }
        }
        let _ = child.wait();
        let _ = app.emit("tmux-detached", session);
    });
    Ok(())
}

#[tauri::command]
pub fn send_keys(state: State<Tmux>, session: String, pane: String, data: String) -> Result<(), String> {
    let mut clients = state.clients.lock().unwrap();
    let stdin = clients.get_mut(&session).ok_or("session not attached")?;
    stdin.write_all(send_keys_hex(&pane, data.as_bytes()).as_bytes()).map_err(|e| e.to_string())
}

/// Sets this client's size, so the window fits the view while you use it.
#[tauri::command]
pub fn resize(state: State<Tmux>, session: String, cols: u16, rows: u16) -> Result<(), String> {
    let mut clients = state.clients.lock().unwrap();
    let stdin = clients.get_mut(&session).ok_or("session not attached")?;
    stdin.write_all(format!("refresh-client -C {cols}x{rows}\n").as_bytes()).map_err(|e| e.to_string())
}
