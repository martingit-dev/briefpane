mod card;
mod git;
mod log;
mod proc;
mod state;
mod tmux;
mod transcript;

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;
use tauri::ipc::Channel;
use tauri::plugin::{Builder as PluginBuilder, TauriPlugin};
use tauri::{AppHandle, Emitter, Runtime, State, Url};

/// `$HOME`, or `/` when unset so paths built on it stay absolute.
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Keeps the window on the app. A clicked link, say in an agent's answer,
/// opens in the system browser instead of replacing the app with the page.
fn links_outside<R: Runtime>() -> TauriPlugin<R> {
    PluginBuilder::new("links-outside")
        .on_navigation(|_, url: &Url| {
            let app = url.scheme() == "tauri"
                || matches!(
                    url.host_str(),
                    Some("localhost" | "tauri.localhost" | "127.0.0.1")
                );
            if !app && matches!(url.scheme(), "http" | "https" | "mailto") {
                let _ = proc::spawn(Command::new("xdg-open").arg(url.as_str()));
            }
            app
        })
        .build()
}

// Every command runs off the main thread (`async`) and is logged by
// `log::cmd` with its duration and outcome.

#[tauri::command(async)]
fn list_panes() -> Result<Vec<tmux::Pane>, String> {
    log::cmd("list_panes", tmux::list_panes)
}

#[tauri::command(async)]
fn capture(pane: String) -> Result<tmux::Snapshot, String> {
    log::cmd("capture", || tmux::capture(&pane))
}

#[tauri::command(async)]
fn pane_size(pane: String) -> Result<tmux::Size, String> {
    log::cmd("pane_size", || tmux::pane_size(&pane))
}

#[tauri::command(async)]
fn new_terminal(session: Option<String>, path: String) -> Result<String, String> {
    log::cmd("new_terminal", || {
        tmux::new_terminal(session.as_deref(), &path)
    })
}

#[tauri::command(async)]
fn attach(app: AppHandle, tmux: State<tmux::Tmux>, session: String) -> Result<(), String> {
    let notify = move |n| {
        let _ = match n {
            tmux::Notice::PanesChanged => app.emit("panes-changed", ()),
            tmux::Notice::Detached(session) => app.emit("tmux-detached", session),
        };
    };
    log::cmd("attach", || tmux.attach(notify, &session))
}

#[tauri::command(async)]
fn send_keys(
    tmux: State<tmux::Tmux>,
    session: String,
    pane: String,
    data: String,
) -> Result<(), String> {
    log::cmd("send_keys", || tmux.send_keys(&session, &pane, &data))
}

#[tauri::command(async)]
fn resize(tmux: State<tmux::Tmux>, session: String, cols: u16, rows: u16) -> Result<(), String> {
    log::cmd("resize", || tmux.resize(&session, cols, rows))
}

#[tauri::command(async)]
fn watch(
    tmux: State<tmux::Tmux>,
    session: String,
    pane: String,
    output: Channel,
) -> Result<u64, String> {
    log::cmd("watch", || tmux.watch(&session, &pane, output))
}

#[tauri::command(async)]
fn unwatch(tmux: State<tmux::Tmux>, id: u64) -> Result<(), String> {
    log::cmd("unwatch", || tmux.unwatch(id))
}

/// The agent's latest answer as markdown, `null` when there is none.
#[tauri::command(async)]
fn brief(cache: State<transcript::Transcripts>, pane: String) -> Result<Option<String>, String> {
    log::cmd("brief", || Ok(transcript::brief(&cache, &pane)))
}

/// Uncommitted changes in the repo containing `path`, old and new text per
/// file. Empty outside a repo; an error when git fails.
#[tauri::command(async)]
fn changes(repos: State<git::Repos>, path: String) -> Result<Vec<git::FileChange>, String> {
    log::cmd("changes", || repos.changes(&path).map_err(String::from))
}

/// The task card for the repo and branch at `path`; `null` when there is none.
#[tauri::command(async)]
fn card(cards: State<card::Cards>, path: String) -> Result<Value, String> {
    log::cmd("card", || cards.card(&path))
}

#[tauri::command(async)]
fn approve_plan(path: String) -> Result<(), String> {
    log::cmd("approve_plan", || card::approve_plan(&path))
}

#[tauri::command(async)]
fn start_review(
    session: String,
    path: String,
    agent: String,
    builder: String,
) -> Result<String, String> {
    log::cmd("start_review", || {
        card::start_review(&session, &path, &agent, &builder)
    })
}

#[tauri::command(async)]
fn dispose(path: String, id: String, verdict: String) -> Result<(), String> {
    log::cmd("dispose", || card::dispose(&path, &id, &verdict))
}

/// The frontend's errors and timings, into the same log. Not itself logged
/// as a `cmd`: that would double every line it writes.
#[tauri::command(async)]
fn log(level: String, msg: String, data: Option<Value>) {
    log::web(&level, &msg, data);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    log::start();
    tauri::Builder::default()
        .plugin(links_outside())
        .manage(tmux::Tmux::default())
        .manage(transcript::Transcripts::default())
        .manage(git::Repos::default())
        .manage(card::Cards::default())
        .invoke_handler(tauri::generate_handler![
            list_panes,
            capture,
            pane_size,
            new_terminal,
            attach,
            send_keys,
            resize,
            watch,
            unwatch,
            brief,
            changes,
            card,
            approve_plan,
            start_review,
            dispose,
            log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
