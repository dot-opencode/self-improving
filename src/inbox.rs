//! Inbox jsonl streaming: a native port of the awk pipeline `inbox_stream`.

use crate::store::Ctx;
use crate::util::*;

/// Tolerates `"key": "v"` (spaced JSON) as well as the compact `"key":"v"`.
/// Mirrors the awk `jget`: first match of `"name"[ \t]*:[ \t]*"`, then the
/// string body with `\n \t \r` -> space and other escapes reduced to the char.
fn jget(s: &str, name: &str) -> String {
    let needle = format!("\"{name}\"");
    let bytes = s.as_bytes();
    let mut from = 0;
    while let Some(off) = s[from..].find(&needle) {
        let start = from + off;
        let mut i = start + needle.len();
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b':' {
            i += 1;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'"' {
                let body = &s[i + 1..];
                let mut out = String::new();
                let mut it = body.chars();
                while let Some(c) = it.next() {
                    if c == '\\' {
                        let n = it.next().unwrap_or('\0');
                        if n == '\0' {
                            break;
                        }
                        out.push(if matches!(n, 'n' | 't' | 'r') { ' ' } else { n });
                    } else if c == '"' {
                        break;
                    } else {
                        out.push(c);
                    }
                }
                return out;
            }
        }
        // advance past this start (needle begins with an ASCII quote)
        from = start + 1;
    }
    String::new()
}

pub struct Filter<'a> {
    pub compact: bool,
    pub since: &'a str,
    pub ack: &'a str,
    pub kind: &'a str,
    pub tier: &'a str,
    pub limit: usize,
}

/// Filtered inbox events, globally sorted by arrival key (stable). Each item is
/// `(key, payload)`; payload is the raw line (raw mode) or the display line.
/// The key is `ats` when present, else `ts`. `limit` keeps the oldest N events,
/// extended to every event sharing the Nth event's key.
pub fn stream(ctx: &Ctx, f: &Filter) -> Vec<(String, String)> {
    let files = glob_suffix(&format!("{}/inbox", ctx.pdir), ".jsonl");
    let mut rows: Vec<(String, String)> = Vec::new();
    for file in files {
        let Some(text) = read_text(&file) else {
            continue;
        };
        for line in text.split('\n') {
            if line.chars().all(is_space) {
                continue;
            }
            let ts = jget(line, "ts");
            let kind = jget(line, "kind");
            let tier = jget(line, "tier");
            let mut key = jget(line, "ats");
            if key.is_empty() {
                key = ts.clone();
            }
            if !f.since.is_empty() && ts.as_str() < f.since {
                continue;
            }
            if !f.ack.is_empty() && key.as_str() <= f.ack {
                continue;
            }
            if !f.kind.is_empty() && kind != f.kind {
                continue;
            }
            if !f.tier.is_empty() && tier != f.tier {
                continue;
            }
            if !f.compact {
                rows.push((key, line.to_string()));
                continue;
            }
            let project = jget(line, "project");
            let text = if kind == "failure" {
                format!("{}: {}", jget(line, "tool"), jget(line, "head"))
            } else {
                let mut t = jget(line, "snippet");
                let prev = jget(line, "prev_tool");
                if !prev.is_empty() {
                    t = format!("{t}  (after {prev})");
                }
                t
            };
            let label = if tier.is_empty() {
                kind.clone()
            } else {
                format!("{kind}/{tier}")
            };
            rows.push((key, format!("{ts}  {label}  {project}  {text}")));
        }
    }
    rows.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    if f.limit > 0 {
        let mut out = Vec::new();
        let mut last = String::new();
        for (n, row) in rows.into_iter().enumerate() {
            if n >= f.limit && row.0 != last {
                break;
            }
            last = row.0.clone();
            out.push(row);
        }
        return out;
    }
    rows
}
