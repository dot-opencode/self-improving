//! `learn grep <pattern> [grep opts]`: case-insensitive recursive search over
//! candidates/applied/rejected, `path:line:text` output like `grep -r -n -i`.
//!
//! Native for the common option subset (-F -E -G -w -x -v -l -L -c -h -H -o and
//! combinations; -i -n -r -R -s are accepted as no-ops since they are implied).
//! Any other option is rejected with a clear error (no system `grep` fallback).

use regex::Regex;
use std::fs;

/// Ok(exit code: 0 = match, 1 = none, 2 = bad regex) or Err(unsupported option message).
pub fn run(pdir: &str, opts: &[String], pattern: &str) -> Result<i32, String> {
    let mut fixed = false;
    let mut ere = false;
    let (mut word, mut line_x, mut invert) = (false, false, false);
    let (mut list, mut list_non, mut count, mut only) = (false, false, false, false);
    let mut with_name = true;
    for o in opts {
        let b = o.as_bytes();
        if b.len() < 2 || b[0] != b'-' || b[1] == b'-' {
            return Err(unsupported(o));
        }
        for c in &b[1..] {
            match c {
                b'F' => fixed = true,
                b'E' => ere = true,
                b'G' => {}
                b'w' => word = true,
                b'x' => line_x = true,
                b'v' => invert = true,
                b'l' => list = true,
                b'L' => list_non = true,
                b'c' => count = true,
                b'o' => only = true,
                b'h' => with_name = false,
                b'H' => with_name = true,
                b'i' | b'n' | b'r' | b'R' | b's' => {}
                _ => return Err(unsupported(o)),
            }
        }
    }
    let mut pat = if fixed {
        regex::escape(pattern)
    } else if ere {
        pattern.to_string()
    } else {
        bre_to_ere(pattern)
    };
    if line_x {
        pat = format!("^(?:{pat})$");
    } else if word {
        pat = format!(r"(?:^|\W)(?:{pat})(?:\W|$)");
    }
    let re = match Regex::new(&format!("(?i){pat}")) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("grep: {e}");
            return Ok(2);
        }
    };
    let mut any = false;
    for d in ["candidates", "applied", "rejected"] {
        let mut files = Vec::new();
        walk(&format!("{pdir}/{d}"), d, &mut files);
        files.sort();
        for (path, full) in files {
            let Ok(bytes) = fs::read(&full) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let mut n = 0;
            let mut out: Vec<String> = Vec::new();
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 && line.is_empty() && i == text.split('\n').count() - 1 {
                    break;
                }
                let hit = re.is_match(line) != invert;
                if !hit {
                    continue;
                }
                n += 1;
                if only && !invert {
                    for m in re.find_iter(line) {
                        out.push(fmt_line(&path, i + 1, m.as_str(), with_name));
                    }
                } else {
                    out.push(fmt_line(&path, i + 1, line, with_name));
                }
            }
            if n > 0 {
                any = true;
            }
            if list {
                if n > 0 {
                    println!("{path}");
                }
            } else if list_non {
                if n == 0 {
                    println!("{path}");
                }
            } else if count {
                if with_name {
                    println!("{path}:{n}");
                } else {
                    println!("{n}");
                }
            } else {
                for l in out {
                    println!("{l}");
                }
            }
        }
    }
    Ok(if any { 0 } else { 1 })
}

fn unsupported(o: &str) -> String {
    format!("unsupported grep option '{o}' (supported: -F -E -G -w -x -v -l -L -c -o -h -H; -i -n -r are always on)")
}

fn fmt_line(path: &str, no: usize, text: &str, with_name: bool) -> String {
    if with_name {
        format!("{path}:{no}:{text}")
    } else {
        format!("{no}:{text}")
    }
}

fn walk(full: &str, rel: &str, out: &mut Vec<(String, String)>) {
    let Ok(md) = fs::metadata(full) else { return };
    if md.is_dir() {
        if let Ok(rd) = fs::read_dir(full) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                walk(&format!("{full}/{n}"), &format!("{rel}/{n}"), out);
            }
        }
    } else if md.is_file() {
        out.push((rel.to_string(), full.to_string()));
    }
}

/// POSIX basic regex -> Rust regex syntax.
fn bre_to_ere(p: &str) -> String {
    let c: Vec<char> = p.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut at_start = true;
    while i < c.len() {
        let ch = c[i];
        match ch {
            '\\' if i + 1 < c.len() => {
                let n = c[i + 1];
                i += 2;
                match n {
                    '(' | ')' | '{' | '}' | '|' | '+' | '?' => {
                        out.push(n);
                        at_start = n == '(' || n == '|';
                    }
                    _ => {
                        out.push('\\');
                        out.push(n);
                        at_start = false;
                    }
                }
                continue;
            }
            '(' | ')' | '{' | '}' | '|' | '+' | '?' => {
                out.push('\\');
                out.push(ch);
            }
            '*' if at_start => out.push_str("\\*"),
            '[' => {
                out.push('[');
                i += 1;
                if i < c.len() && c[i] == '^' {
                    out.push('^');
                    i += 1;
                }
                if i < c.len() && c[i] == ']' {
                    out.push_str("\\]");
                    i += 1;
                }
                while i < c.len() && c[i] != ']' {
                    if c[i] == '[' && i + 1 < c.len() && c[i + 1] == ':' {
                        // POSIX class, copy through ":]"
                        while i < c.len() && !(c[i] == ']' && c[i - 1] == ':') {
                            out.push(c[i]);
                            i += 1;
                        }
                        if i < c.len() {
                            out.push(']');
                            i += 1;
                        }
                        continue;
                    }
                    if c[i] == '\\' || c[i] == '[' {
                        out.push('\\');
                    }
                    out.push(c[i]);
                    i += 1;
                }
                out.push(']');
                i += 1;
                at_start = false;
                continue;
            }
            '^' => {
                out.push('^');
                i += 1;
                continue; // still "at start" for a following '*'
            }
            _ => out.push(ch),
        }
        at_start = false;
        i += 1;
    }
    out
}
