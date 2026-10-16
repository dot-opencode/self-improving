//! Small shared helpers: process exit, string/date helpers, temp files, paths.

use std::ffi::CString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Path of the dream lock held by this process (set by `dream::lock_acquire`).
pub static HELD_LOCK: OnceLock<PathBuf> = OnceLock::new();
/// Same path as a C string for the async-signal handler.
pub static HELD_LOCK_C: OnceLock<CString> = OnceLock::new();

pub fn first_line_of(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some(text.split('\n').next().unwrap_or("").to_string())
}

/// Remove the dream lock, but only when it still holds our PID.
pub fn release_lock_if_ours() {
    if let Some(p) = HELD_LOCK.get() {
        let pid = std::process::id().to_string();
        if first_line_of(p).as_deref() == Some(pid.as_str()) {
            let _ = fs::remove_file(p);
        }
    }
}

pub fn exit_with(code: i32) -> ! {
    let _ = io::stdout().flush();
    release_lock_if_ours();
    std::process::exit(code)
}

pub fn die(msg: &str) -> ! {
    let _ = io::stdout().flush();
    eprintln!("learn: {msg}");
    exit_with(1)
}

pub fn warn(msg: &str) {
    eprintln!("learn: {msg}");
}

pub fn read_text(path: impl AsRef<Path>) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

pub fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r')
}

/// Collapse newlines/tabs to spaces and trim, so values stay on one line.
pub fn oneline(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if matches!(c, '\n' | '\r' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect();
    t.trim_matches(is_space).to_string()
}

pub fn is_uint(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

pub fn valid_slug(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || !(b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) {
        return false;
    }
    b.iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

pub fn valid_key(s: &str) -> bool {
    let Some((area, sym)) = s.split_once('.') else {
        return false;
    };
    !area.is_empty()
        && area.bytes().all(|c| c.is_ascii_lowercase())
        && !sym.is_empty()
        && sym
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

pub fn valid_owner(s: &str) -> bool {
    for p in ["script", "skill", "new-skill", "agents", "config", "hint"] {
        if let Some(rest) = s.strip_prefix(p).and_then(|r| r.strip_prefix(':')) {
            return !rest.is_empty();
        }
    }
    false
}

/// Same substring semantics as the bash `case " $2 " in *" $1 "*)`.
pub fn in_list(word: &str, list: &str) -> bool {
    format!(" {list} ").contains(&format!(" {word} "))
}

pub fn require_slug(what: &str, v: &str) {
    if !valid_slug(v) {
        die(&format!(
            "invalid {what} '{v}': use lowercase letters, digits, and hyphens"
        ));
    }
}

pub fn valid_calendar_date(d: &str) -> bool {
    let b = d.as_bytes();
    if b.len() != 10 {
        return false;
    }
    for (i, c) in b.iter().enumerate() {
        let ok = if i == 4 || i == 7 {
            *c == b'-'
        } else {
            c.is_ascii_digit()
        };
        if !ok {
            return false;
        }
    }
    let year: i64 = d[0..4].parse().unwrap_or(0);
    let month: i64 = d[5..7].parse().unwrap_or(0);
    let day: i64 = d[8..10].parse().unwrap_or(0);
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    day >= 1 && day <= max
}

/// Days-from-civil for a validated YYYY-MM-DD date.
pub fn days_from_civil(date: &str) -> i64 {
    let mut y: i64 = date[0..4].parse().unwrap_or(0);
    let m: i64 = date[5..7].parse().unwrap_or(1);
    let d: i64 = date[8..10].parse().unwrap_or(1);
    if m <= 2 {
        y -= 1;
    }
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn today() -> String {
    match std::env::var("LEARN_TODAY") {
        Ok(v) if !v.is_empty() => {
            if !valid_calendar_date(&v) {
                die(&format!("invalid LEARN_TODAY: {v}"));
            }
            v
        }
        _ => local_date(),
    }
}

fn local_date() -> String {
    // SAFETY: plain libc time calls with properly initialised out-parameters.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!(
            "{:04}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        )
    }
}

pub fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// `basename(1)` semantics.
pub fn basename(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.is_empty() {
            String::new()
        } else {
            "/".into()
        };
    }
    match t.rfind('/') {
        Some(i) => t[i + 1..].to_string(),
        None => t.to_string(),
    }
}

pub fn dirname(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.is_empty() { ".".into() } else { "/".into() };
    }
    match t.rfind('/') {
        None => ".".into(),
        Some(0) => "/".into(),
        Some(i) => {
            let d = t[..i].trim_end_matches('/');
            if d.is_empty() {
                "/".into()
            } else {
                d.to_string()
            }
        }
    }
}

pub fn canon_dir(p: &str) -> Option<String> {
    fs::canonicalize(p)
        .ok()
        .map(|c| c.to_string_lossy().into_owned())
}

pub fn is_dir(p: &str) -> bool {
    fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

pub fn is_file(p: &str) -> bool {
    fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

pub fn is_symlink(p: &str) -> bool {
    fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

pub fn exists(p: &str) -> bool {
    fs::metadata(p).is_ok()
}

/// Resolve symlinks of the final component (hop-capped) and canonicalise the
/// directory. Mirrors the bash `canonical_path` (the file need not exist).
pub fn canonical_path(orig: &str) -> Result<String, String> {
    let mut path = orig.to_string();
    let mut hops = 0;
    while is_symlink(&path) {
        hops += 1;
        if hops > 40 {
            return Err(format!("too many levels of symbolic links: {orig}"));
        }
        let dir = canon_dir(&dirname(&path)).ok_or_else(String::new)?;
        let target = fs::read_link(&path)
            .map_err(|_| String::new())?
            .to_string_lossy()
            .into_owned();
        path = if target.starts_with('/') {
            target
        } else {
            format!("{dir}/{target}")
        };
    }
    let dir = canon_dir(&dirname(&path)).ok_or_else(String::new)?;
    Ok(format!("{}/{}", dir, basename(&path)))
}

fn rand_name(n: usize) -> String {
    static CTR: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos
        ^ ((std::process::id() as u64) << 32)
        ^ CTR
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut s = String::new();
    for _ in 0..n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.push(chars[(x % chars.len() as u64) as usize] as char);
    }
    s
}

pub fn mktemp_in(dir: &str, prefix: &str) -> io::Result<PathBuf> {
    let mut last = io::Error::other("mktemp failed");
    for _ in 0..100 {
        let p = Path::new(dir).join(format!("{prefix}{}", rand_name(6)));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&p)
        {
            Ok(_) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Temp file next to its destination so the final rename is atomic.
pub fn mk_tmp(dir: &str) -> PathBuf {
    match mktemp_in(dir, ".learn.") {
        Ok(p) => {
            let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o644));
            p
        }
        Err(_) => die(&format!("cannot create temporary file in {dir}")),
    }
}

pub fn write_atomic(dir: &str, dest: &str, contents: &[u8]) {
    let tmp = mk_tmp(dir);
    if fs::write(&tmp, contents).is_err() {
        let _ = fs::remove_file(&tmp);
        die(&format!("cannot write {dest}"));
    }
    mv(&tmp, Path::new(dest));
}

pub fn mv(from: &Path, to: &Path) {
    if let Err(e) = fs::rename(from, to) {
        let _ = fs::remove_file(from);
        die(&format!("mv: cannot move to {}: {e}", to.display()));
    }
}

pub fn mkdir_p(p: &str) {
    if let Err(e) = fs::create_dir_all(p) {
        die(&format!("mkdir: cannot create {p}: {e}"));
    }
}

/// Non-hidden regular-or-otherwise existing entries of `dir` ending in `suffix`,
/// sorted bytewise (like a shell glob `dir/*suffix`).
pub fn glob_suffix(dir: &str, suffix: &str) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !name.ends_with(suffix) {
                continue;
            }
            let full = format!("{dir}/{name}");
            if exists(&full) {
                v.push(full);
            }
        }
    }
    v.sort();
    v
}

/// Count of lines like `grep -c ''` on a multi-line string.
pub fn count_lines(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count()
    }
}
