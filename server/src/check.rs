//! The compiler's check, `zeru check --json <file>`, run in a child process
//! whose output a thread of its own reads, so requests are never kept
//! waiting. Each line it prints is one error.

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use crate::Event;

/// A check under way: which one it is, and its process.
pub struct Running {
    pub generation: u64,
    child: Arc<Mutex<Child>>,
}

impl Running {
    pub fn kill(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

/// What a check found, as the compiler printed it.
pub struct Report {
    pub root: PathBuf,
    pub generation: u64,
    pub errors: Vec<Value>,
}

/// `text` as `zeru fmt` lays it out, or None when it cannot: a syntax
/// error, or a compiler with no `fmt`.
pub fn format(zeru: &Path, text: &str) -> Option<String> {
    let mut child = Command::new(zeru)
        .args(["fmt", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(text.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then_some(String::from_utf8(output.stdout).ok()?)
}

/// Check `file` with the compiler `zeru`, from the file's directory, and
/// send the report as an event once the compiler is done.
pub fn spawn(
    zeru: &Path,
    file: &Path,
    generation: u64,
    events: Sender<Event>,
) -> io::Result<Running> {
    let dir = file.parent().unwrap_or(Path::new("/"));
    let mut child = Command::new(zeru)
        .args(["check", "--json"])
        .arg(file.file_name().unwrap_or_default())
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no stdout"))?;
    let child = Arc::new(Mutex::new(child));
    let waited = Arc::clone(&child);
    let root = file.to_path_buf();
    thread::spawn(move || {
        let lines = BufReader::new(stdout).lines().map_while(Result::ok);
        let errors = lines
            .filter_map(|line| serde_json::from_str(&line).ok())
            .collect();
        if let Ok(mut child) = waited.lock() {
            let _ = child.wait();
        }
        let _ = events.send(Event::Checked(Report {
            root,
            generation,
            errors,
        }));
    });
    Ok(Running { generation, child })
}

/// The compiler: `path` if given, else `ZERU_PATH`, else `zeru` on `PATH`,
/// else `~/.zeru/bin/zeru`.
pub fn find_zeru(path: Option<&str>) -> Option<PathBuf> {
    let given = path
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("ZERU_PATH").map(PathBuf::from));
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("zeru"));
    let home = std::env::var_os("HOME").map(|home| Path::new(&home).join(".zeru/bin/zeru"));
    given
        .into_iter()
        .chain(on_path)
        .chain(home)
        .find(|path| path.is_file())
}

/// The standard library: `ZERU_STD_PATH`, else the `std` in the closest
/// directory above the compiler that has one, as the compiler finds it.
pub fn find_std(zeru: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ZERU_STD_PATH") {
        return Some(path.into());
    }
    let exe = zeru?.canonicalize().ok()?;
    exe.ancestors()
        .map(|dir| dir.join("std"))
        .find(|std| std.join("builtin.zr").is_file())
}
