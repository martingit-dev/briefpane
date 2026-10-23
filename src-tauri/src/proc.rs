//! Every child process starts here, so each one is logged as a `spawn`.

use std::fmt;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use serde_json::json;

use crate::log;

#[derive(Debug)]
pub enum Error {
    /// The program could not be started at all.
    Spawn {
        program: String,
        err: std::io::Error,
    },
    /// It ran and exited non-zero.
    Failed { program: String, stderr: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Spawn { program, err } => write!(f, "{program}: {err}"),
            Error::Failed { program, stderr } if stderr.is_empty() => {
                write!(f, "{program} failed")
            }
            Error::Failed { stderr, .. } => f.write_str(stderr),
        }
    }
}

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.to_string()
    }
}

fn logged(program: &str, args: &[&str], start: Instant, ok: bool) {
    // `git -C <dir>` says nothing about what ran; the subcommand does.
    let args = match args {
        ["-C", _, rest @ ..] => rest,
        _ => args,
    };
    let first: Vec<&str> = args.iter().take(2).copied().collect();
    log::event(
        "spawn",
        json!({ "program": program, "args": first, "ms": log::ms(start), "ok": ok }),
    );
}

/// Runs `program` to completion and returns its stdout.
pub fn run(program: &str, args: &[&str]) -> Result<Vec<u8>, Error> {
    run_with_input(program, args, None)
}

/// Like [`run`], writing `input` to the child's stdin first.
pub fn run_with_input(
    program: &str,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<Vec<u8>, Error> {
    let start = Instant::now();
    let spawn_err = |err| Error::Spawn {
        program: program.into(),
        err,
    };
    let result = (|| {
        let mut child = Command::new(program)
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(spawn_err)?;
        if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
            // A writer thread keeps a large input from deadlocking against
            // a child that fills its stdout pipe before reading on.
            let data = data.to_vec();
            std::thread::spawn(move || stdin.write_all(&data));
        }
        let out = child.wait_with_output().map_err(spawn_err)?;
        if !out.status.success() {
            return Err(Error::Failed {
                program: program.into(),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(out.stdout)
    })();
    logged(program, args, start, result.is_ok());
    result
}

/// Starts a long-lived child; the caller owns it from here.
pub fn spawn(cmd: &mut Command) -> Result<Child, Error> {
    let start = Instant::now();
    let program = cmd.get_program().to_string_lossy().into_owned();
    let args: Vec<String> = cmd
        .get_args()
        .take(2)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let child = cmd.spawn().map_err(|err| Error::Spawn {
        program: program.clone(),
        err,
    });
    logged(&program, &args, start, child.is_ok());
    child
}

pub fn text(out: Vec<u8>) -> String {
    String::from_utf8_lossy(&out).into_owned()
}
