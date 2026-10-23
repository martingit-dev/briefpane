//! Bridges tmux to the window. One control-mode client per tmux session
//! streams the output of watched panes over channels; listing and capture
//! use plain `tmux` calls. The app is one more tmux client: with tmux's
//! default `window-size latest`, whichever client you used last sets a
//! window's size.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;
use tauri::ipc::{Channel, InvokeResponseBody};
use tmux_cc::{Coalescer, Event, Parser, send_keys_hex};

use crate::log;
use crate::proc;
use crate::state::{AgentState, state_of};

/// Live control-mode clients by session name. Each carries a serial so a
/// reader thread only removes its own client, not a newer one for a
/// recreated session of the same name. Nothing under this lock blocks:
/// writes go to the client's writer thread through `tx`.
#[derive(Default)]
pub struct Tmux {
    clients: Arc<Mutex<HashMap<String, Client>>>,
    serial: AtomicU64,
    watch_id: AtomicU64,
}

#[derive(Clone)]
struct Client {
    serial: u64,
    /// `$N`: safe on a control-mode command line, unlike a session name.
    session_id: String,
    /// Command lines for the client's stdin, written by its own thread.
    tx: Sender<Vec<u8>>,
    streams: Arc<Mutex<Streams>>,
}

/// Which panes of one client stream where. tmux sends every pane's output
/// to a control client until told otherwise, so an unwatched pane is
/// paused the first time it writes. Paused, not `off`: tmux stops reading a
/// pane that every client has turned off, which blocks the agent in it.
#[derive(Default)]
struct Streams {
    watchers: HashMap<String, Vec<(u64, Channel)>>,
    off: HashSet<String>,
}

impl Streams {
    fn turn(&mut self, tx: &Sender<Vec<u8>>, pane: &str, on: bool) {
        let flag = if on { "continue" } else { "pause" };
        let _ = tx.send(format!("refresh-client -A '{pane}:{flag}'\n").into_bytes());
        if on {
            self.off.remove(pane);
        } else {
            self.off.insert(pane.to_string());
        }
    }

    /// Removes one watcher; the pane's last one turns it off. Dropping the
    /// channel ends it on the webview side.
    fn remove(&mut self, tx: &Sender<Vec<u8>>, id: u64) -> bool {
        let Some(pane) = self
            .watchers
            .iter()
            .find(|(_, ws)| ws.iter().any(|(w, _)| *w == id))
            .map(|(p, _)| p.clone())
        else {
            return false;
        };
        let ws = self.watchers.get_mut(&pane).expect("found above");
        ws.retain(|(w, _)| *w != id);
        if ws.is_empty() {
            self.watchers.remove(&pane);
            self.turn(tx, &pane, false);
        }
        true
    }
}

/// The `$N` id of the session named exactly `name`. Every target is built
/// from it: tmux target lookup matches name prefixes and reads `$N` names
/// as ids, and prints an empty id for a missing session.
pub fn session_id(name: &str) -> Result<String, String> {
    let list = tmux(&["list-sessions", "-F", "#{session_id}\t#{session_name}"])?;
    id_of(&list, name).ok_or_else(|| format!("no session {name:?}"))
}

/// The id of the session named exactly `name`, from `list-sessions -F
/// '#{session_id}\t#{session_name}'`.
fn id_of(list: &str, name: &str) -> Option<String> {
    list.lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(_, n)| *n == name)
        .map(|(id, _)| id.to_string())
}

/// Named so a stuck thread can be told apart in `/proc/<pid>/task/*/comm`.
fn spawn_named(name: &str, f: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(f);
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A tmux pane id such as `%12`. Anything else is refused: the id is
/// written into a control-mode command line, where `;` starts a new command.
pub fn pane_id_ok(pane: &str) -> Result<(), String> {
    match pane.strip_prefix('%') {
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => Ok(()),
        _ => Err(format!("not a pane id: {pane:?}")),
    }
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

#[derive(Serialize, Debug, PartialEq)]
pub struct Size {
    width: u16,
    height: u16,
}

pub fn tmux(args: &[&str]) -> Result<String, String> {
    Ok(proc::text(proc::run("tmux", args)?))
}

const PANE_FORMAT: &str = "#{session_name}\t#{window_id}\t#{window_index}\t#{window_name}\t#{pane_id}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_width}\t#{pane_height}\t#{pane_title}\t#{pane_pid}";

/// One line of `list-panes -F PANE_FORMAT`.
#[derive(Debug, PartialEq)]
pub struct RawPane<'a> {
    session: &'a str,
    window_id: &'a str,
    window_index: u32,
    window_name: &'a str,
    pane_id: &'a str,
    command: &'a str,
    path: &'a str,
    width: u16,
    height: u16,
    title: &'a str,
    pid: &'a str,
}

pub fn parse_pane(line: &str) -> Option<RawPane<'_>> {
    let mut f = line.split('\t');
    let mut next = || f.next();
    let pane = RawPane {
        session: next()?,
        window_id: next()?,
        window_index: next()?.parse().unwrap_or(0),
        window_name: next()?,
        pane_id: next()?,
        command: next()?,
        path: next()?,
        width: next()?.parse().unwrap_or(80),
        height: next()?.parse().unwrap_or(24),
        title: next()?,
        pid: next()?,
    };
    next().is_none().then_some(pane)
}

const AGENTS: [&str; 2] = ["claude", "codex"];

const SHELLS: [&str; 5] = ["bash", "zsh", "fish", "sh", "dash"];

/// What a pane is for, judged by its processes: an agent you talk to, a
/// service you watch, or a bare shell.
#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
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

/// The agent among some processes' argv lists. Only argv[0] and argv[1]
/// count: `claude`, or `node /path/to/codex`. The command name alone
/// misleads: Codex runs as `node`.
pub fn agent_in(argvs: &[Vec<String>]) -> Option<&'static str> {
    argvs.iter().flat_map(|a| a.iter().take(2)).find_map(|arg| {
        let base = arg.rsplit('/').next().unwrap_or("");
        AGENTS.into_iter().find(|a| base == *a)
    })
}

pub fn kind_of(agent: Option<&str>, has_children: bool, command: &str) -> Kind {
    match agent {
        Some(_) => Kind::Agent,
        None if !has_children && SHELLS.contains(&command) => Kind::Terminal,
        None => Kind::Service,
    }
}

fn argv(pid: &str) -> Vec<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    raw.split(|&b| b == 0)
        .take(2)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}

pub fn list_panes() -> Result<Vec<Pane>, String> {
    let out = tmux(&["list-panes", "-a", "-F", PANE_FORMAT])?;
    Ok(out
        .lines()
        .filter_map(parse_pane)
        .map(|p| {
            // The pane's shell and its direct children.
            let kids = children(p.pid);
            let argvs: Vec<Vec<String>> = std::iter::once(p.pid)
                .chain(kids.iter().map(String::as_str))
                .map(argv)
                .collect();
            let agent = agent_in(&argvs);
            let kind = kind_of(agent, !kids.is_empty(), p.command);
            Pane {
                session: p.session.into(),
                window_id: p.window_id.into(),
                window_index: p.window_index,
                window_name: p.window_name.into(),
                pane_id: p.pane_id.into(),
                command: p.command.into(),
                path: p.path.into(),
                width: p.width,
                height: p.height,
                title: p.title.into(),
                agent: agent.map(String::from),
                kind,
                state: agent.and_then(|_| state_of(p.pane_id)),
            }
        })
        .collect())
}

/// What the pane shows now, with colours, so the view starts filled before
/// live output arrives.
pub fn capture(pane: &str) -> Result<Snapshot, String> {
    let history = tmux(&[
        "capture-pane",
        "-p",
        "-e",
        "-S",
        "-5000",
        "-E",
        "-1",
        "-t",
        pane,
    ])?;
    let text = tmux(&["capture-pane", "-p", "-e", "-t", pane])?;
    let cur = tmux(&[
        "display-message",
        "-p",
        "-t",
        pane,
        "#{cursor_x} #{cursor_y} #{pane_width} #{pane_height}",
    ])?;
    let mut it = cur.split_whitespace().map(|n| n.parse().unwrap_or(0));
    let mut next = || it.next().unwrap_or(0);
    Ok(Snapshot {
        history,
        text,
        cursor_x: next(),
        cursor_y: next(),
        width: next(),
        height: next(),
    })
}

pub fn parse_size(out: &str) -> Option<Size> {
    let mut it = out.split_whitespace().map(str::parse);
    match (it.next(), it.next(), it.next()) {
        (Some(Ok(width)), Some(Ok(height)), None) => Some(Size { width, height }),
        _ => None,
    }
}

pub fn pane_size(pane: &str) -> Result<Size, String> {
    pane_id_ok(pane)?;
    let out = tmux(&[
        "display-message",
        "-p",
        "-t",
        pane,
        "#{pane_width} #{pane_height}",
    ])?;
    parse_size(&out).ok_or_else(|| format!("no size for {pane}: {out:?}"))
}

/// Opens a window running the default shell in `session`, or in `shells` when
/// no session is given. Returns the new pane's id.
pub fn new_terminal(session: Option<&str>, path: &str) -> Result<String, String> {
    let home = crate::home().to_string_lossy().into_owned();
    let path = if path == "~" { home.as_str() } else { path };
    let window = |id: &str| {
        tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-c",
            path,
            "-t",
            &format!("{id}:"),
        ])
    };
    let pane = match session {
        Some(s) => window(&session_id(s)?)?,
        None => match session_id("shells") {
            Ok(id) => window(&id)?,
            Err(_) => tmux(&[
                "new-session",
                "-d",
                "-s",
                "shells",
                "-P",
                "-F",
                "#{pane_id}",
                "-c",
                path,
            ])?,
        },
    };
    Ok(pane.trim().to_string())
}

/// What a control client tells the window besides pane output.
pub enum Notice {
    /// Windows or pane sizes changed: list the panes again.
    PanesChanged,
    /// The session's client ended.
    Detached(String),
}

/// A frame waits at most this long behind a steady stream of lines.
const FLUSH_EVERY: Duration = Duration::from_millis(16);
/// Or until this many bytes are held.
const FLUSH_BYTES: usize = 256 * 1024;

impl Tmux {
    /// Starts a control client for `session`. Idempotent per session.
    pub fn attach(
        &self,
        notify: impl Fn(Notice) + Send + 'static,
        session: &str,
    ) -> Result<(), String> {
        if lock(&self.clients).contains_key(session) {
            return Ok(());
        }
        // Matched by name against the session list: tmux target lookup reads
        // `$N` as an id and prints an empty id for a missing session, and
        // `attach -t ""` would join some other session.
        let session_id = session_id(session)?;
        let mut child = proc::spawn(
            Command::new("tmux")
                .args(["-C", "attach-session", "-t", &session_id])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        )?;
        let (Some(stdout), Some(mut stdin)) = (child.stdout.take(), child.stdin.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("tmux control client has no pipes".into());
        };
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let streams = Arc::new(Mutex::new(Streams::default()));
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        {
            let mut clients = lock(&self.clients);
            // Another attach won the race while this one spawned.
            if clients.contains_key(session) {
                drop(clients);
                let _ = child.kill();
                let _ = child.wait();
                return Ok(());
            }
            clients.insert(
                session.to_string(),
                Client {
                    serial,
                    session_id: session_id.clone(),
                    tx: tx.clone(),
                    streams: Arc::clone(&streams),
                },
            );
        }

        log::event(
            "client",
            json!({ "session": session, "id": session_id, "serial": serial }),
        );
        let who = session.to_string();
        spawn_named("bp-write", move || {
            for line in rx {
                if let Err(e) = stdin.write_all(&line) {
                    log::event(
                        "error",
                        json!({ "msg": "tmux client write", "session": who, "err": e.to_string() }),
                    );
                    break;
                }
            }
        });

        let registry = Arc::clone(&self.clients);
        let session = session.to_string();
        spawn_named("bp-read", move || {
            read(&notify, &session, stdout, &streams, &tx);
            // The session is gone: forget the client so the next attach starts afresh.
            {
                let mut clients = lock(&registry);
                if clients.get(&session).is_some_and(|c| c.serial == serial) {
                    clients.remove(&session);
                }
            }
            let _ = child.wait();
            log::event(
                "client-end",
                json!({ "session": session, "serial": serial }),
            );
            notify(Notice::Detached(session));
        });
        Ok(())
    }

    /// A copy of the session's handles, so no caller holds the map lock.
    fn client(&self, session: &str) -> Result<Client, String> {
        lock(&self.clients)
            .get(session)
            .cloned()
            .ok_or_else(|| "session not attached".into())
    }

    pub fn send_keys(&self, session: &str, pane: &str, data: &str) -> Result<(), String> {
        pane_id_ok(pane)?;
        let tx = self.client(session)?.tx;
        tx.send(send_keys_hex(pane, data.as_bytes()).into_bytes())
            .map_err(|_| "tmux client closed".into())
    }

    /// Sets this client's size, so the window fits the view while you use
    /// it. `switch-client` first makes the app the latest client: once
    /// another terminal has typed, tmux sizes the window to that terminal
    /// and ignores `refresh-client -C` alone.
    pub fn resize(&self, session: &str, cols: u16, rows: u16) -> Result<(), String> {
        let c = self.client(session)?;
        let cmd = format!(
            "switch-client -t {}\nrefresh-client -C {cols}x{rows}\n",
            c.session_id
        );
        c.tx.send(cmd.into_bytes())
            .map_err(|_| "tmux client closed".into())
    }

    /// Streams `pane`'s output to `output` as raw byte frames.
    pub fn watch(&self, session: &str, pane: &str, output: Channel) -> Result<u64, String> {
        pane_id_ok(pane)?;
        let Client { tx, streams, .. } = self.client(session)?;
        let id = self.watch_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut s = lock(&streams);
        let first = !s.watchers.contains_key(pane);
        s.watchers
            .entry(pane.to_string())
            .or_default()
            .push((id, output));
        if first {
            s.turn(&tx, pane, true);
        }
        Ok(id)
    }

    pub fn unwatch(&self, id: u64) -> Result<(), String> {
        let all: Vec<_> = lock(&self.clients)
            .values()
            .map(|c| (c.tx.clone(), Arc::clone(&c.streams)))
            .collect();
        for (tx, streams) in all {
            if lock(&streams).remove(&tx, id) {
                return Ok(());
            }
        }
        // Its session already detached, which dropped the watcher.
        Ok(())
    }
}

/// Bytes and frames sent per pane since the last `stream` line.
#[derive(Default)]
struct Rates {
    since: Option<Instant>,
    panes: HashMap<String, (usize, usize)>,
}

impl Rates {
    fn add(&mut self, pane: &str, bytes: usize) {
        self.since.get_or_insert_with(Instant::now);
        let r = self.panes.entry(pane.to_string()).or_default();
        r.0 += bytes;
        r.1 += 1;
    }

    /// One line per pane once a second while busy. The window opens at the
    /// first frame, so a burst shorter than a second is reported by the next
    /// frame after it.
    fn report(&mut self) {
        let Some(since) = self.since else { return };
        let secs = since.elapsed().as_secs_f64();
        if secs < 1.0 {
            return;
        }
        let per_s = secs;
        for (pane, (bytes, frames)) in self.panes.drain() {
            log::event(
                "stream",
                json!({
                    "pane": pane,
                    "bytes": bytes,
                    "frames": frames,
                    "secs": (secs * 1000.0).round() / 1000.0,
                    "bytes_s": (bytes as f64 / per_s).round(),
                    "frames_s": (frames as f64 / per_s * 10.0).round() / 10.0,
                }),
            );
        }
        self.since = None;
    }
}

/// Reads one control client until tmux closes it, sending each watched
/// pane's output as one frame per read burst.
fn read(
    notify: &impl Fn(Notice),
    session: &str,
    stdout: impl std::io::Read,
    streams: &Mutex<Streams>,
    tx: &Sender<Vec<u8>>,
) {
    let mut reader = BufReader::with_capacity(64 * 1024, stdout);
    let mut parser = Parser::new();
    let mut pending = Coalescer::default();
    let mut rates = Rates::default();
    let mut last_flush = Instant::now();
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                log::event(
                    "error",
                    json!({ "msg": "tmux client read", "session": session, "err": e.to_string() }),
                );
                break;
            }
        }
        if buf.last() == Some(&b'\n') {
            buf.pop();
        }
        match parser.line(&buf) {
            Some(Event::Output { pane, data }) => pending.push(pane, data),
            Some(Event::Exit(_)) => break,
            Some(
                Event::WindowAdd { .. } | Event::WindowClose { .. } | Event::WindowRenamed { .. },
            ) => {
                notify(Notice::PanesChanged);
            }
            // Pane sizes changed: a split, or another client took over the size.
            Some(Event::Other(line)) if line.starts_with("%layout-change ") => {
                notify(Notice::PanesChanged);
            }
            _ => {}
        }
        // Nothing left in the buffer means tmux has paused: the burst is over.
        if pending.bytes() > 0
            && (reader.buffer().is_empty()
                || pending.bytes() >= FLUSH_BYTES
                || last_flush.elapsed() >= FLUSH_EVERY)
        {
            flush(&mut pending, streams, tx, &mut rates);
            last_flush = Instant::now();
        }
    }
}

fn flush(
    pending: &mut Coalescer,
    streams: &Mutex<Streams>,
    tx: &Sender<Vec<u8>>,
    rates: &mut Rates,
) {
    let mut sends = Vec::new();
    {
        let mut s = lock(streams);
        for (pane, data) in pending.take() {
            match s.watchers.get(&pane) {
                Some(ws) => sends.push((pane, data, ws.clone())),
                None if !s.off.contains(&pane) => s.turn(tx, &pane, false),
                None => {}
            }
        }
    }
    // Channels are sent outside the lock: a send evaluates script in the webview.
    let mut dead = Vec::new();
    for (pane, data, ws) in sends {
        rates.add(&pane, data.len());
        let (last, rest) = ws.split_last().expect("watchers are never empty");
        for (id, ch) in rest {
            if ch.send(InvokeResponseBody::Raw(data.clone())).is_err() {
                dead.push(*id);
            }
        }
        if last.1.send(InvokeResponseBody::Raw(data)).is_err() {
            dead.push(last.0);
        }
    }
    if !dead.is_empty() {
        let mut s = lock(streams);
        for id in dead {
            log::event(
                "error",
                json!({ "msg": "stream channel closed", "watch": id }),
            );
            s.remove(tx, id);
        }
    }
    rates.report();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_by_exact_name() {
        let list = "$1\t$0\n$0\tkeep\n$2\tkeeper\n";
        assert_eq!(id_of(list, "$0").as_deref(), Some("$1"));
        assert_eq!(id_of(list, "keep").as_deref(), Some("$0"));
        assert_eq!(id_of(list, "kee"), None);
        assert_eq!(id_of(list, ""), None);
    }

    #[test]
    fn pane_ids_only() {
        assert!(pane_id_ok("%0").is_ok());
        assert!(pane_id_ok("%123").is_ok());
        for bad in ["", "%", "0", "%1 ; run-shell x", "%1\n", "%a"] {
            assert!(pane_id_ok(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn pane_line() {
        let line = "heya\t@3\t2\tapi\t%12\tnode\t/home/mg/heya\t211\t56\t✳ Fix login\t4242";
        assert_eq!(
            parse_pane(line),
            Some(RawPane {
                session: "heya",
                window_id: "@3",
                window_index: 2,
                window_name: "api",
                pane_id: "%12",
                command: "node",
                path: "/home/mg/heya",
                width: 211,
                height: 56,
                title: "✳ Fix login",
                pid: "4242",
            })
        );
        // An empty title still counts as a field.
        assert!(parse_pane("s\t@1\t0\tw\t%1\tbash\t/\t80\t24\t\t1").is_some());
        assert_eq!(parse_pane("s\t@1\t0\tw\t%1\tbash\t/\t80\t24\tt"), None);
        assert_eq!(
            parse_pane("s\t@1\t0\tw\t%1\tbash\t/\t80\t24\tt\t1\textra"),
            None
        );
        let odd = parse_pane("s\t@1\tx\tw\t%1\tbash\t/\t?\t?\tt\t1").unwrap();
        assert_eq!((odd.window_index, odd.width, odd.height), (0, 80, 24));
    }

    fn argvs(list: &[&[&str]]) -> Vec<Vec<String>> {
        list.iter()
            .map(|a| a.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn agents_by_argv() {
        assert_eq!(
            agent_in(&argvs(&[&["-bash"], &["claude", "--resume"]])),
            Some("claude")
        );
        assert_eq!(
            agent_in(&argvs(&[
                &["zsh"],
                &["node", "/usr/lib/node_modules/@openai/codex/bin/codex"]
            ])),
            Some("codex")
        );
        assert_eq!(
            agent_in(&argvs(&[&["/home/mg/.local/bin/claude"]])),
            Some("claude")
        );
        // Only argv[0] and argv[1] count; a later mention is just an argument.
        assert_eq!(agent_in(&argvs(&[&["vim", "notes.md", "claude"]])), None);
        assert_eq!(
            agent_in(&argvs(&[&["node", "server.js"], &["claude-wrapper"]])),
            None
        );
        assert_eq!(agent_in(&[]), None);
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of(Some("claude"), true, "node"), Kind::Agent);
        assert_eq!(kind_of(None, false, "bash"), Kind::Terminal);
        assert_eq!(kind_of(None, true, "bash"), Kind::Service);
        assert_eq!(kind_of(None, false, "node"), Kind::Service);
    }

    #[test]
    fn sizes() {
        assert_eq!(
            parse_size("211 56\n"),
            Some(Size {
                width: 211,
                height: 56
            })
        );
        assert_eq!(parse_size("211\n"), None);
        assert_eq!(parse_size("a b"), None);
    }

    /// Run with `cargo test -- --ignored`. The server lives under its own
    /// `TMUX_TMPDIR`, so the user's tmux server is never reached.
    #[test]
    #[ignore = "starts a scratch tmux server"]
    fn streams_only_watched_panes() {
        let dir = std::env::temp_dir().join(format!("bp-tmux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: this test runs alone (`--ignored`), before any thread reads the env.
        unsafe {
            std::env::set_var("TMUX_TMPDIR", &dir);
            std::env::remove_var("TMUX");
        }
        let loop_of = |w: &str| format!("while :; do echo {w}; sleep 0.05; done");
        tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "t",
            "-x",
            "80",
            "-y",
            "24",
            &loop_of("AAA"),
        ])
        .unwrap();
        tmux(&["new-window", "-d", "-t", "t:", &loop_of("BBB")]).unwrap();
        let ids = tmux(&["list-panes", "-s", "-t", "t", "-F", "#{pane_id}"]).unwrap();
        let ids: Vec<&str> = ids.lines().collect();
        let (a, b) = (ids[0], ids[1]);

        let t = Tmux::default();
        t.attach(|_| {}, "t").unwrap();
        let got = Arc::new(Mutex::new(Vec::<u8>::new()));
        let frames = Arc::new(AtomicU64::new(0));
        let sink = {
            let (got, frames) = (Arc::clone(&got), Arc::clone(&frames));
            Channel::new(move |body| {
                if let InvokeResponseBody::Raw(bytes) = body {
                    lock(&got).extend_from_slice(&bytes);
                    frames.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            })
        };
        let id = t.watch("t", a, sink).unwrap();
        std::thread::sleep(Duration::from_millis(1000));
        let text = String::from_utf8_lossy(&lock(&got)).into_owned();
        assert!(text.contains("AAA"), "{text:?}");
        assert!(!text.contains("BBB"), "{text:?}");
        let n = frames.load(Ordering::Relaxed);
        assert!((5..=40).contains(&n), "{n} frames");

        // Unwatched: nothing more arrives.
        t.unwatch(id).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let after = frames.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(frames.load(Ordering::Relaxed), after);

        // A channel that fails is dropped, and its pane turned off.
        let id = t
            .watch(
                "t",
                b,
                Channel::new(|_| Err(tauri::Error::FailedToReceiveMessage)),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let c = t.client("t").unwrap();
        let s = lock(&c.streams);
        assert!(s.watchers.is_empty());
        assert!(s.off.contains(a) && s.off.contains(b));
        drop(s);
        t.unwatch(id).unwrap();

        // A burst streams through to its last line.
        tmux(&["new-window", "-d", "-t", "t:", "bash --norc --noprofile"]).unwrap();
        let panes = tmux(&["list-panes", "-s", "-t", "t", "-F", "#{pane_id}"]).unwrap();
        let burst = panes.lines().last().unwrap().to_string();
        let got = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = {
            let got = Arc::clone(&got);
            Channel::new(move |body| {
                if let InvokeResponseBody::Raw(bytes) = body {
                    lock(&got).extend_from_slice(&bytes);
                }
                Ok(())
            })
        };
        t.watch("t", &burst, sink).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        t.send_keys("t", &burst, "seq 1 200000; echo DONE-BURST\r")
            .unwrap();
        let done = || String::from_utf8_lossy(&lock(&got)).contains("DONE-BURST\r\n");
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end && !done() {
            std::thread::sleep(Duration::from_millis(100));
        }
        let text = String::from_utf8_lossy(&lock(&got)).into_owned();
        let tail = &text[text.len().saturating_sub(200)..];
        assert!(done(), "{} bytes, tail {tail:?}", text.len());

        let _ = tmux(&["kill-server"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
