//! One JSON line per event in `$BRIEFPANE_LOG`, default
//! `/tmp/briefpane/briefpane.jsonl`. Format: `docs/perf-plan.md`.
//! Logging never fails the caller: a line that cannot be written is dropped.

use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::{Value, json};

/// Rotated at start above this size.
const MAX_BYTES: u64 = 8 << 20;
/// A successful command faster than this is not logged: keystrokes and
/// cache hits would otherwise fill the log.
const QUIET_MS: f64 = 20.0;

fn file() -> &'static Mutex<Option<File>> {
    static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
    FILE.get_or_init(|| {
        // Tests run the logged code paths; they must not write to the app's log.
        if cfg!(test) {
            return Mutex::new(None);
        }
        let path = std::env::var_os("BRIEFPANE_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp/briefpane/briefpane.jsonl"));
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // /tmp is often RAM: one previous log is kept, the rest goes.
        if path.metadata().is_ok_and(|m| m.len() > MAX_BYTES) {
            let _ = std::fs::rename(&path, path.with_extension("jsonl.1"));
        }
        Mutex::new(OpenOptions::new().create(true).append(true).open(path).ok())
    })
}

/// Local wall-clock time to the millisecond, e.g. `2026-10-23T20:40:01.123`.
fn now() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

/// One line: `t`, `src` and `kind` first, then `fields` (an object).
fn line(src: &str, kind: &str, fields: Value) -> String {
    let mut text = format!(
        "{{\"t\":{},\"src\":{},\"kind\":{}",
        Value::from(now()),
        Value::from(src),
        Value::from(kind)
    );
    if let Value::Object(f) = fields {
        for (k, v) in f
            .into_iter()
            .filter(|(k, _)| !matches!(k.as_str(), "t" | "src" | "kind"))
        {
            text.push(',');
            text.push_str(&Value::String(k).to_string());
            text.push(':');
            text.push_str(&v.to_string());
        }
    }
    text.push_str("}\n");
    text
}

pub fn write(src: &str, kind: &str, fields: Value) {
    let text = line(src, kind, fields);
    let Ok(mut f) = file().lock() else {
        return;
    };
    if let Some(f) = f.as_mut() {
        let _ = f.write_all(text.as_bytes());
    }
}

pub fn event(kind: &str, fields: Value) {
    write("rust", kind, fields);
}

/// A failure the app recovers from, so it would otherwise go unseen.
pub fn error(msg: &str, err: impl Display) {
    event("error", json!({ "msg": msg, "err": err.to_string() }));
}

/// Milliseconds since `start`, to 0.1 ms.
pub fn ms(start: Instant) -> f64 {
    (start.elapsed().as_secs_f64() * 10_000.0).round() / 10.0
}

/// Runs a command body and logs its name, duration and outcome.
pub fn cmd<T, E: Display>(name: &str, body: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let start = Instant::now();
    let out = body();
    let ms = ms(start);
    match &out {
        Ok(_) if ms < QUIET_MS => {}
        Ok(_) => event("cmd", json!({ "name": name, "ms": ms, "ok": true })),
        Err(e) => event(
            "cmd",
            json!({ "name": name, "ms": ms, "ok": false, "err": e.to_string() }),
        ),
    }
    out
}

pub fn start() {
    event(
        "start",
        json!({ "version": env!("CARGO_PKG_VERSION"), "pid": std::process::id() }),
    );
}

/// The frontend's line. `perf` carries `msg` as `name` with `data`'s fields
/// beside it, matching the plan's `{"kind":"perf","name":..,"ms":..}`;
/// every other level keeps `msg` and nests `data`.
pub fn web(level: &str, msg: &str, data: Option<Value>) {
    let fields = match (level, data) {
        ("perf", Some(Value::Object(mut d))) => {
            d.insert("name".into(), msg.into());
            Value::Object(d)
        }
        ("perf", _) => json!({ "name": msg }),
        (_, Some(d)) => json!({ "msg": msg, "data": d }),
        (_, None) => json!({ "msg": msg }),
    };
    write("web", level, fields);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_shape() {
        let got = line(
            "rust",
            "cmd",
            json!({ "name": "changes", "ms": 12.4, "ok": true, "t": "x" }),
        );
        assert!(got.starts_with("{\"t\":\""), "{got}");
        assert!(
            got.ends_with(
                ",\"src\":\"rust\",\"kind\":\"cmd\",\"ms\":12.4,\"name\":\"changes\",\"ok\":true}\n"
            ),
            "{got}"
        );
        let v: Value = serde_json::from_str(&got).unwrap();
        assert_eq!(v["kind"], "cmd");
    }
}
