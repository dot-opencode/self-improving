//! Thin wrappers over the external `git` binary.

use crate::util::exit_with;
use std::io::Write;
use std::process::{Command, Stdio};

fn build(store: Option<&str>, args: &[&str]) -> Command {
    let mut c = Command::new("git");
    if let Some(s) = store {
        c.arg("-C").arg(s);
    }
    c.args(args);
    c
}

fn code_of(st: std::io::Result<std::process::ExitStatus>) -> i32 {
    match st {
        Ok(s) => s.code().unwrap_or(128 + signal_of(&s)),
        Err(_) => 127,
    }
}

pub fn signal_of(s: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    s.signal().unwrap_or(0)
}

/// Run git; `quiet` sends stdout+stderr to /dev/null, otherwise both are inherited.
pub fn code(store: Option<&str>, args: &[&str], quiet: bool) -> i32 {
    let _ = std::io::stdout().flush();
    let mut c = build(store, args);
    if quiet {
        c.stdout(Stdio::null()).stderr(Stdio::null());
    }
    code_of(c.status())
}

/// Run git with stderr discarded and stdout inherited... (used for plain checks).
pub fn code_silent_out(store: Option<&str>, args: &[&str]) -> i32 {
    let mut c = build(store, args);
    c.stdout(Stdio::null()).stderr(Stdio::null());
    code_of(c.status())
}

/// Capture stdout (trailing newlines trimmed). stderr is discarded when
/// `stderr_null`, otherwise inherited.
pub fn out(store: Option<&str>, args: &[&str], stderr_null: bool) -> (i32, String) {
    let _ = std::io::stdout().flush();
    let mut c = build(store, args);
    c.stdout(Stdio::piped());
    if stderr_null {
        c.stderr(Stdio::null());
    }
    match c.output() {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout)
                .trim_end_matches('\n')
                .to_string();
            (o.status.code().unwrap_or(128 + signal_of(&o.status)), s)
        }
        Err(_) => (127, String::new()),
    }
}

/// `set -e` semantics: a failing command ends the script with its exit status.
pub fn must(rc: i32) {
    if rc != 0 {
        exit_with(rc);
    }
}
