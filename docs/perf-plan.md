# Performance and quality pass

Source: the review of `a18e115`. Polling stays, with change detection; push
(tmux `-B` subscriptions, inotify) is a later step.

## Contract between backend and frontend

Every command is `#[tauri::command(async)]` or `async fn`. Names not listed
keep their current signature.

- `attach(session)`: starts the control client. Idempotent.
- `watch(session, pane, output: Channel<ArrayBuffer>) -> u64`: streams that
  pane's output as raw byte frames, coalesced per read burst. Resumes the pane
  with `refresh-client -A '%N:continue'`. Replaces the `pane-output` event.
- `unwatch(id)`: stops that stream; the last watcher pauses the pane
  (`%N:pause`). Never `off`: tmux stops reading a pane all clients turned off.
- `pane_size(pane) -> { width, height }`: one `display-message`.
- `capture(pane)`: unchanged, called once after the size settles.
- `log(level, msg, data?)`: the frontend's errors and timings into the log.
- `card`, `brief`, `changes`: same results; Rust returns a cached value while
  the source file's (len, mtime) or the repo's status output is unchanged.

## Log

One JSON line per event in `$BRIEFPANE_LOG`, default `/tmp/briefpane/briefpane.jsonl`:

    {"t":"2026-10-23T20:40:01.123","src":"rust","kind":"cmd","name":"changes","ms":12.4,"ok":true}
    {"t":"...","src":"rust","kind":"stream","pane":"%12","bytes_s":5400,"frames_s":30}
    {"t":"...","src":"web","kind":"error","msg":"...","data":{...}}
    {"t":"...","src":"web","kind":"perf","name":"pane-open","ms":180}

Kinds: `start`, `cmd` (every command: name, ms, ok, err), `spawn` (every
child process: program, ms), `stream` (per watched pane, once a second while
busy), `error`, `perf`.

## Expectations (each one is checked)

1. No command runs on the main thread; no command over 50 ms in the log at idle.
2. Idle with an agent pane selected: at most 25 process spawns a minute: the 3 s pane list is 20 of them until tmux pushes pane changes.
3. Opening a pane: one `capture`, size polled with `pane_size`; under 300 ms in the log.
4. Output frames are raw bytes; only watched panes stream.
5. A stalled tmux client never blocks another session's keys (no IO under the map lock).
6. Changes tab: unchanged files are not re-highlighted; data fetched only while the tab is open; the diff code is lazy-loaded.
7. Pure logic has unit tests: porcelain -z parsing (renames, copies), agent and kind detection, last-answer extraction, pane-line parsing.
8. `pnpm lint`, `cargo fmt --check`, `cargo clippy -D warnings`, unit tests and `pnpm e2e` pass.

## Known issues

- Output that arrives while a pane's opening snapshot is taken can be painted
  twice, so typing in the first few hundred ms can garble a line until the
  next redraw. Fix: request the snapshot through the control client, where
  tmux orders the reply with `%output`.
