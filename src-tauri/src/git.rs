//! The repo's uncommitted changes, old and new text per file. A call costs
//! one `git status` while nothing changed: files are reread only when the
//! status output or a listed file's size or mtime differs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::Serialize;

use crate::proc::{self, Error};

/// Files past this size are listed but not diffed.
const MAX_FILE: u64 = 256 * 1024;
const MAX_FILES: usize = 30;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FileChange {
    name: String,
    /// `M`, `A`, `D` or `?` (untracked), from `git status --porcelain`.
    status: String,
    old: Option<String>,
    new: Option<String>,
}

/// One `git status --porcelain -z` entry.
#[derive(Debug, PartialEq)]
pub struct Entry {
    /// The two status letters, `XY`.
    pub code: String,
    pub path: String,
    /// The source of a rename or copy.
    pub orig: Option<String>,
}

/// Parses `git status --porcelain -z`. A rename or copy (`R` or `C` in
/// either column) is followed by a second NUL-terminated field, its source.
pub fn parse_porcelain(raw: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut parts = raw.split(|&b| b == 0).filter(|e| !e.is_empty());
    while let Some(e) = parts.next() {
        if e.len() < 4 {
            continue;
        }
        let code = String::from_utf8_lossy(&e[..2]).into_owned();
        let orig = code
            .contains(['R', 'C'])
            .then(|| {
                parts
                    .next()
                    .map(|o| String::from_utf8_lossy(o).into_owned())
            })
            .flatten();
        entries.push(Entry {
            code,
            path: String::from_utf8_lossy(&e[3..]).into_owned(),
            orig,
        });
    }
    entries
}

fn status_of(code: &str) -> &'static str {
    match code {
        "??" => "?",
        c if c.contains('D') => "D",
        c if c.contains('A') => "A",
        _ => "M",
    }
}

/// Splits `git cat-file --batch` output into one object per request:
/// `<oid> <type> <size>\n<bytes>\n`, or `<name> missing\n`.
pub fn parse_batch(mut out: &[u8], n: usize) -> Vec<Option<Vec<u8>>> {
    let mut objects = Vec::with_capacity(n);
    for _ in 0..n {
        let Some(eol) = out.iter().position(|&b| b == b'\n') else {
            break;
        };
        let header = String::from_utf8_lossy(&out[..eol]).into_owned();
        out = &out[eol + 1..];
        let size = header
            .rsplit(' ')
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|_| header.split(' ').count() == 3 && !header.ends_with(" missing"));
        match size {
            Some(size) if size <= out.len() => {
                objects.push(Some(out[..size].to_vec()));
                out = out.get(size + 1..).unwrap_or_default();
            }
            _ => objects.push(None),
        }
    }
    objects.resize(n, None);
    objects
}

fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let mut all = vec!["-C", dir.to_str().unwrap_or(".")];
    all.extend(args);
    proc::run("git", &all)
}

type Stamp = Option<(u64, SystemTime)>;

fn stamp(path: &Path) -> Stamp {
    let m = path.metadata().ok()?;
    Some((m.len(), m.modified().ok()?))
}

struct Snapshot {
    status: Vec<u8>,
    stamps: Vec<Stamp>,
    head: Vec<Stamp>,
    files: Vec<FileChange>,
}

#[derive(Clone)]
struct Repo {
    root: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
}

impl Repo {
    /// Stamps of what a commit rewrites: HEAD, the index, the branch ref,
    /// packed refs and a reftable's table list. A partial commit leaves the status and the work tree as
    /// they were but moves HEAD, so the old side of every diff changes.
    fn head(&self) -> Vec<Stamp> {
        let head = std::fs::read_to_string(self.git_dir.join("HEAD")).unwrap_or_default();
        let branch = head
            .strip_prefix("ref: ")
            .map(|r| self.common_dir.join(r.trim()));
        [
            Some(self.git_dir.join("HEAD")),
            Some(self.git_dir.join("index")),
            branch,
            Some(self.common_dir.join("packed-refs")),
            Some(self.common_dir.join("reftable/tables.list")),
            Some(self.git_dir.join("reftable/tables.list")),
        ]
        .into_iter()
        .map(|p| p.and_then(|p| stamp(&p)))
        .collect()
    }
}

#[derive(Default)]
pub struct Repos {
    roots: Mutex<HashMap<String, Repo>>,
    last: Mutex<HashMap<PathBuf, Snapshot>>,
}

impl Repos {
    /// The repo containing `path`; `None` outside a repo.
    fn root(&self, path: &str) -> Result<Option<Repo>, Error> {
        if let Some(r) = self
            .roots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(path)
        {
            return Ok(Some(r.clone()));
        }
        let args = [
            "rev-parse",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
        ];
        match git(Path::new(path), &args) {
            Ok(out) => {
                let out = proc::text(out);
                let mut lines = out.lines().map(PathBuf::from);
                let (Some(root), Some(git_dir), Some(common)) =
                    (lines.next(), lines.next(), lines.next())
                else {
                    return Ok(None);
                };
                // `--git-common-dir` may be relative to `path`.
                let repo = Repo {
                    common_dir: Path::new(path).join(common),
                    root,
                    git_dir,
                };
                self.roots
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(path.into(), repo.clone());
                Ok(Some(repo))
            }
            Err(Error::Failed { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Uncommitted changes in the repo containing `path`. Empty outside a
    /// repo; an error when git itself fails.
    pub fn changes(&self, path: &str) -> Result<Vec<FileChange>, Error> {
        let Some(repo) = self.root(path)? else {
            return Ok(vec![]);
        };
        let root = repo.root.clone();
        let status = git(
            &root,
            &["status", "--porcelain", "-z", "--untracked-files=all"],
        )
        .inspect_err(|_| {
            // The root may be gone or moved; look it up afresh next time.
            self.roots
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(path);
        })?;
        let entries: Vec<Entry> = parse_porcelain(&status)
            .into_iter()
            .take(MAX_FILES)
            .collect();
        let stamps: Vec<Stamp> = entries.iter().map(|e| stamp(&root.join(&e.path))).collect();
        let head = repo.head();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = last.get(&root)
            && s.status == status
            && s.stamps == stamps
            && s.head == head
        {
            return Ok(s.files.clone());
        }
        drop(last);
        let files = read(&root, &entries, &stamps)?;
        last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        last.insert(
            root,
            Snapshot {
                status,
                stamps,
                head,
                files: files.clone(),
            },
        );
        Ok(files)
    }
}

fn read(root: &Path, entries: &[Entry], stamps: &[Stamp]) -> Result<Vec<FileChange>, Error> {
    let text = |b: Vec<u8>| String::from_utf8(b).ok();
    let small = |s: &Stamp| s.is_none_or(|(len, _)| len <= MAX_FILE);
    // Old text comes from HEAD in one `cat-file --batch`; names with a
    // newline cannot be asked for on its line protocol.
    let wants: Vec<Option<&str>> = entries
        .iter()
        .zip(stamps)
        .map(|(e, s)| {
            let status = status_of(&e.code);
            let name = e.orig.as_deref().unwrap_or(&e.path);
            (small(s) && status != "?" && status != "A" && !name.contains('\n')).then_some(name)
        })
        .collect();
    let request: String = wants
        .iter()
        .flatten()
        .map(|n| format!("HEAD:{n}\n"))
        .collect();
    let mut olds = if request.is_empty() {
        Vec::new()
    } else {
        let args = ["-C", root.to_str().unwrap_or("."), "cat-file", "--batch"];
        let out = proc::run_with_input("git", &args, Some(request.as_bytes()))?;
        parse_batch(&out, wants.iter().flatten().count())
    }
    .into_iter();
    Ok(entries
        .iter()
        .zip(stamps)
        .zip(&wants)
        .map(|((e, s), want)| {
            let status = status_of(&e.code);
            let old = want.and_then(|_| olds.next().flatten()).and_then(text);
            let new = (small(s) && status != "D")
                .then(|| std::fs::read(root.join(&e.path)).ok().and_then(text))
                .flatten();
            FileChange {
                name: e.path.clone(),
                status: status.into(),
                old,
                new,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A partial commit keeps the status line and the file as they were but
    /// moves HEAD; the cached diff must pick up the new old side.
    #[test]
    fn partial_commit_refreshes_the_cache() {
        let dir = std::env::temp_dir().join(format!("bp-git-partial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sh = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .output()
                .unwrap();
            assert!(ok.status.success(), "{args:?}: {ok:?}");
            String::from_utf8(ok.stdout).unwrap()
        };
        sh(&["init", "-q"]);
        std::fs::write(dir.join("f.txt"), "a\nb\nc\n").unwrap();
        sh(&["add", "."]);
        sh(&["commit", "-qm", "one"]);
        std::fs::write(dir.join("f.txt"), "A\nB\nc\n").unwrap();

        let repos = Repos::default();
        let path = dir.to_str().unwrap();
        assert_eq!(
            repos.changes(path).unwrap()[0].old.as_deref(),
            Some("a\nb\nc\n")
        );

        // Commit only the first line, as `git add -p && git commit` would.
        let blob = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(["hash-object", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(b"A\nb\nc\n")?;
                c.wait_with_output()
            })
            .unwrap();
        let oid = String::from_utf8(blob.stdout).unwrap();
        sh(&[
            "update-index",
            "--cacheinfo",
            &format!("100644,{},f.txt", oid.trim()),
        ]);
        sh(&["commit", "-qm", "two"]);

        assert_eq!(
            repos.changes(path).unwrap()[0].old.as_deref(),
            Some("A\nb\nc\n")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn entry(code: &str, path: &str, orig: Option<&str>) -> Entry {
        Entry {
            code: code.into(),
            path: path.into(),
            orig: orig.map(String::from),
        }
    }

    #[test]
    fn porcelain_entries() {
        let raw = b" M src/a.rs\0?? new file.txt\0R  b.rs\0old b.rs\0C  c2.rs\0c.rs\0 D gone.rs\0A  added.rs\0";
        assert_eq!(
            parse_porcelain(raw),
            vec![
                entry(" M", "src/a.rs", None),
                entry("??", "new file.txt", None),
                entry("R ", "b.rs", Some("old b.rs")),
                entry("C ", "c2.rs", Some("c.rs")),
                entry(" D", "gone.rs", None),
                entry("A ", "added.rs", None),
            ]
        );
        assert_eq!(parse_porcelain(b""), vec![]);
    }

    #[test]
    fn worktree_rename_carries_source() {
        assert_eq!(
            parse_porcelain(b" R to\0from\0 M x\0"),
            vec![entry(" R", "to", Some("from")), entry(" M", "x", None)]
        );
    }

    #[test]
    fn statuses() {
        let got: Vec<&str> = ["??", " M", "MM", " D", "D ", "A ", "AM", "R ", "C "]
            .iter()
            .map(|c| status_of(c))
            .collect();
        assert_eq!(got, ["?", "M", "M", "D", "D", "A", "A", "M", "M"]);
    }

    #[test]
    fn batch_objects() {
        let out = b"abc blob 5\nhello\nHEAD:x missing\ndef blob 0\n\nfff blob 3\na\nb\n";
        assert_eq!(
            parse_batch(out, 4),
            vec![
                Some(b"hello".to_vec()),
                None,
                Some(vec![]),
                Some(b"a\nb".to_vec())
            ]
        );
        assert_eq!(parse_batch(b"", 2), vec![None, None]);
    }

    #[test]
    fn changes_in_a_scratch_repo() {
        let dir = std::env::temp_dir().join(format!("bp-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let g = |args: &[&str]| git(&dir, args).unwrap();
        g(&["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        std::fs::write(dir.join("b.txt"), "bee\n").unwrap();
        g(&["add", "."]);
        g(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "x",
        ]);
        std::fs::write(dir.join("a.txt"), "two\n").unwrap();
        g(&["mv", "b.txt", "c.txt"]);
        std::fs::write(dir.join("u.txt"), "new\n").unwrap();
        let repos = Repos::default();
        let got = repos.changes(dir.to_str().unwrap()).unwrap();
        let f = |name: &str, status: &str, old: Option<&str>, new: Option<&str>| FileChange {
            name: name.into(),
            status: status.into(),
            old: old.map(String::from),
            new: new.map(String::from),
        };
        assert_eq!(
            got,
            vec![
                f("a.txt", "M", Some("one\n"), Some("two\n")),
                f("c.txt", "M", Some("bee\n"), Some("bee\n")),
                f("u.txt", "?", None, Some("new\n")),
            ]
        );
        // Same size, new mtime: reread.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("a.txt"), "333\n").unwrap();
        let got = repos.changes(dir.to_str().unwrap()).unwrap();
        assert_eq!(got[0].new.as_deref(), Some("333\n"));
        assert_eq!(repos.changes("/").unwrap(), vec![]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
