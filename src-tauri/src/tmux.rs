//! Bridges tmux to the window. One control-mode client per tmux session
//! streams pane output as events; listing and capture use plain `tmux`
//! calls. Clients attach with `ignore-size` so briefpane never resizes a
//! pane under the terminal that owns it.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::Mutex;

use serde::Serialize;
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
}

#[derive(Serialize, Clone)]
struct Output {
    pane: String,
    data: Vec<u8>,
}

#[derive(Serialize)]
pub struct Snapshot {
    text: String,
    cursor_x: u16,
    cursor_y: u16,
}

fn tmux(args: &[&str]) -> Result<String, String> {
    let out = Command::new("tmux").args(args).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

const PANE_FORMAT: &str = "#{session_name}\t#{window_id}\t#{window_index}\t#{window_name}\t#{pane_id}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_width}\t#{pane_height}";

#[tauri::command]
pub fn list_panes() -> Result<Vec<Pane>, String> {
    let out = tmux(&["list-panes", "-a", "-F", PANE_FORMAT])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() == 9).then(|| Pane {
                session: f[0].into(),
                window_id: f[1].into(),
                window_index: f[2].parse().unwrap_or(0),
                window_name: f[3].into(),
                pane_id: f[4].into(),
                command: f[5].into(),
                path: f[6].into(),
                width: f[7].parse().unwrap_or(80),
                height: f[8].parse().unwrap_or(24),
            })
        })
        .collect())
}

/// What the pane shows now, with colours, so the view starts filled before
/// live output arrives.
#[tauri::command]
pub fn capture(pane: String) -> Result<Snapshot, String> {
    let text = tmux(&["capture-pane", "-p", "-e", "-t", &pane])?;
    let cur = tmux(&["display-message", "-p", "-t", &pane, "#{cursor_x} #{cursor_y}"])?;
    let mut it = cur.split_whitespace().map(|n| n.parse().unwrap_or(0));
    Ok(Snapshot { text, cursor_x: it.next().unwrap_or(0), cursor_y: it.next().unwrap_or(0) })
}

/// Starts streaming a session's panes. Idempotent per session.
#[tauri::command]
pub fn attach(app: AppHandle, state: State<Tmux>, session: String) -> Result<(), String> {
    let mut clients = state.clients.lock().unwrap();
    if clients.contains_key(&session) {
        return Ok(());
    }
    let mut child = Command::new("tmux")
        .args(["-C", "attach-session", "-f", "ignore-size", "-t", &session])
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
