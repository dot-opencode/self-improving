//! Native port of the marker/hint logic of `opencode/learn-core.js`
//! (unicodeize, compile, validateMarkers, validateHints, loadMarkers,
//! matchMarkers). The JS file stays for the OpenCode plugin; keep the two in sync.

use fancy_regex::Regex;
use serde_json::Value;

const WORD: &str = "\\p{L}\\p{N}_";
const MATCH_WINDOW: usize = 400;

/// JS `\b` and `\w` are ASCII-only even with the `u` flag, so learn-core.js
/// rewrites them to Unicode-aware equivalents before compiling; this does the
/// same (verbatim port). Additionally `\d`/`\D` are pinned to ASCII, which is
/// what JS does and what Rust would otherwise widen to all Unicode digits.
pub fn unicodeize(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::new();
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let Some(&n) = chars.get(i + 1) else {
                out.push(c);
                break;
            };
            i += 1;
            if !in_class && n == 'b' {
                out.push_str(&format!(
                    "(?:(?<=[{WORD}])(?![{WORD}])|(?<![{WORD}])(?=[{WORD}]))"
                ));
            } else if !in_class && n == 'B' {
                out.push_str(&format!(
                    "(?:(?<=[{WORD}])(?=[{WORD}])|(?<![{WORD}])(?![{WORD}]))"
                ));
            } else if n == 'w' {
                if in_class {
                    out.push_str(WORD);
                } else {
                    out.push_str(&format!("[{WORD}]"));
                }
            } else if !in_class && n == 'W' {
                out.push_str(&format!("[^{WORD}]"));
            } else if n == 'd' {
                out.push_str(if in_class { "0-9" } else { "[0-9]" });
            } else if !in_class && n == 'D' {
                out.push_str("[^0-9]");
            } else {
                out.push(c);
                out.push(n);
            }
            i += 1;
            continue;
        }
        if c == '[' && !in_class {
            in_class = true;
        } else if c == ']' && in_class {
            in_class = false;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// JS `u`-mode rejects a lone `{`, `}` or `]` outside a character class
/// (Annex B literal fallbacks are disabled); fancy-regex would accept them.
fn js_unicode_syntax_error(source: &str) -> Option<&'static str> {
    let c: Vec<char> = source.chars().collect();
    let mut in_class = false;
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' => i += 1,
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            ']' => return Some("Lone quantifier brackets"),
            '}' if !in_class => return Some("Lone quantifier brackets"),
            '{' if !in_class => {
                let mut j = i + 1;
                let d0 = j;
                while j < c.len() && c[j].is_ascii_digit() {
                    j += 1;
                }
                let mut ok = j > d0;
                if ok && j < c.len() && c[j] == ',' {
                    j += 1;
                    while j < c.len() && c[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                ok = ok && j < c.len() && c[j] == '}';
                if !ok {
                    return Some("Incomplete quantifier");
                }
                i = j;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Compile like `new RegExp(unicodeize(source), "iu")`.
pub fn compile(source: &str) -> Result<Regex, String> {
    if let Some(msg) = js_unicode_syntax_error(source) {
        return Err(format!("Invalid regular expression: /{source}/iu: {msg}"));
    }
    Regex::new(&format!("(?i){}", unicodeize(source))).map_err(|e| {
        format!(
            "Invalid regular expression: /{}/iu: {}",
            source,
            e.to_string().replace('\n', " ")
        )
    })
}

pub struct Compiled {
    pub source: String,
    pub re: Regex,
}

#[derive(Default)]
pub struct Markers {
    pub strong: Vec<Compiled>,
    pub medium: Vec<Compiled>,
    pub weak: Vec<Compiled>,
    pub confirmation: Vec<Compiled>,
}

fn compile_list(list: Option<&Value>) -> Vec<Compiled> {
    let mut out = Vec::new();
    if let Some(Value::Array(items)) = list {
        for it in items {
            if let Value::String(s) = it {
                if s.is_empty() {
                    continue;
                }
                if let Ok(re) = compile(s) {
                    out.push(Compiled {
                        source: s.clone(),
                        re,
                    });
                }
            }
        }
    }
    out
}

/// Missing or unparsable files are `None` (same as `readJson` in JS).
pub fn read_json(path: &str) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Default first, personal appended. Missing/invalid files are ignored.
pub fn load_markers(default_path: &str, personal_path: &str) -> Markers {
    let mut m = Markers::default();
    for src in [read_json(default_path), read_json(personal_path)]
        .into_iter()
        .flatten()
    {
        if !src.is_object() {
            continue;
        }
        let corr = src.get("correction");
        m.strong
            .extend(compile_list(corr.and_then(|c| c.get("strong"))));
        m.medium
            .extend(compile_list(corr.and_then(|c| c.get("medium"))));
        m.weak
            .extend(compile_list(corr.and_then(|c| c.get("weak"))));
        m.confirmation.extend(compile_list(src.get("confirmation")));
    }
    m
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

fn strip_wrapping_quotes(text: &str) -> String {
    let value = js_trim(text);
    let chars: Vec<char> = value.chars().collect();
    if chars.len() >= 2
        && ['"', '\'', '«', '„', '“'].contains(&chars[0])
        && ['"', '\'', '»', '“', '”'].contains(&chars[chars.len() - 1])
    {
        let inner: String = chars[1..chars.len() - 1].iter().collect();
        return js_trim(&inner).to_string();
    }
    value.to_string()
}

/// `stripWrappingQuotes(text).slice(0, 400).toLowerCase()` (400 UTF-16 units).
fn normalize(text: &str) -> String {
    let s = strip_wrapping_quotes(text);
    let mut units = 0;
    let mut cut = String::new();
    for c in s.chars() {
        units += c.len_utf16();
        if units > MATCH_WINDOW {
            break;
        }
        cut.push(c);
    }
    cut.to_lowercase()
}

pub struct Match {
    #[allow(dead_code)]
    pub role: &'static str,
    pub tier: &'static str,
    pub marker: String,
}

/// Order: strong -> medium -> weak -> confirmation.
pub fn match_markers(text: &str, m: &Markers) -> Option<Match> {
    let value = normalize(text);
    if value.is_empty() {
        return None;
    }
    let tiers: [(&'static str, &Vec<Compiled>); 3] = [
        ("strong", &m.strong),
        ("medium", &m.medium),
        ("weak", &m.weak),
    ];
    for (tier, list) in tiers {
        for c in list {
            if c.re.is_match(&value).unwrap_or(false) {
                return Some(Match {
                    role: "correction",
                    tier,
                    marker: c.source.clone(),
                });
            }
        }
    }
    for c in &m.confirmation {
        if c.re.is_match(&value).unwrap_or(false) {
            return Some(Match {
                role: "confirmation",
                tier: "confirmation",
                marker: c.source.clone(),
            });
        }
    }
    None
}

fn js(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn version_error(json: &Value) -> Option<String> {
    let ok = json
        .get("version")
        .and_then(|v| v.as_f64())
        .map(|f| f == 1.0)
        .unwrap_or(false);
    if ok {
        None
    } else {
        let shown = match json.get("version") {
            None => "undefined".to_string(),
            Some(v) => js(v),
        };
        Some(format!("unsupported version: {shown} (expected 1)"))
    }
}

/// `validateMarkers(json)`: error strings; empty means valid.
pub fn validate_markers(json: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if !json.is_object() {
        return vec!["markers must be a JSON object".to_string()];
    }
    if let Some(e) = version_error(json) {
        errors.push(e);
    }
    let mut lists: Vec<(String, Option<&Value>)> =
        vec![("confirmation".into(), json.get("confirmation"))];
    match json.get("correction") {
        Some(c @ (Value::Object(_) | Value::Array(_))) => {
            for tier in ["strong", "medium", "weak"] {
                lists.push((format!("correction.{tier}"), c.get(tier)));
            }
        }
        _ => errors.push("missing 'correction' object".to_string()),
    }
    for (name, list) in lists {
        let Some(list) = list else { continue };
        let Value::Array(items) = list else {
            errors.push(format!("'{name}' must be an array"));
            continue;
        };
        for (i, item) in items.iter().enumerate() {
            match item {
                Value::String(s) if !s.is_empty() => {
                    if let Err(e) = compile(s) {
                        errors.push(format!("'{name}[{i}]' invalid regex {}: {e}", js_str(s)));
                    }
                }
                _ => errors.push(format!("'{name}[{i}]' must be a non-empty string")),
            }
        }
    }
    errors
}

/// `validateHints(json)`: error strings; empty means valid.
pub fn validate_hints(json: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    if !json.is_object() {
        return vec!["hints must be a JSON object".to_string()];
    }
    if let Some(e) = version_error(json) {
        errors.push(e);
    }
    let Some(Value::Array(hints)) = json.get("hints") else {
        errors.push("'hints' must be an array".to_string());
        return errors;
    };
    let mut seen: Vec<&str> = Vec::new();
    for (i, hint) in hints.iter().enumerate() {
        let at = format!("hints[{i}]");
        if !hint.is_object() {
            errors.push(format!("'{at}' must be an object"));
            continue;
        }
        match hint.get("id") {
            Some(Value::String(id)) if !id.is_empty() => {
                if seen.contains(&id.as_str()) {
                    errors.push(format!("'{at}.id' duplicate id {}", js_str(id)));
                } else {
                    seen.push(id);
                }
            }
            _ => errors.push(format!("'{at}.id' must be a non-empty string")),
        }
        let tools_ok = match hint.get("tools") {
            Some(Value::Array(t)) => {
                !t.is_empty()
                    && t.iter()
                        .all(|x| matches!(x, Value::String(s) if !s.is_empty()))
            }
            _ => false,
        };
        if !tools_ok {
            errors.push(format!("'{at}.tools' must be a non-empty array of strings"));
        }
        if !matches!(hint.get("text"), Some(Value::String(s)) if !s.is_empty()) {
            errors.push(format!("'{at}.text' must be a non-empty string"));
        }
        match hint.get("match") {
            Some(Value::String(m)) if !m.is_empty() => {
                if let Err(e) = compile(m) {
                    errors.push(format!("'{at}.match' invalid regex {}: {e}", js_str(m)));
                }
            }
            _ => errors.push(format!("'{at}.match' must be a non-empty string")),
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(strong: &[&str]) -> Markers {
        let v = serde_json::json!({"version":1,"correction":{"strong":strong,"medium":[],"weak":[]},"confirmation":["^(ок|ok)\\b"]});
        Markers {
            strong: compile_list(v.pointer("/correction/strong")),
            confirmation: compile_list(v.get("confirmation")),
            ..Default::default()
        }
    }

    #[test]
    fn cyrillic_word_boundary() {
        let m = mk(&["\\bя про\\b"]);
        assert!(match_markers("Нет, я про другое", &m).is_some());
        assert!(match_markers("поя прокурор", &m).is_none());
    }

    #[test]
    fn quotes_and_confirmation() {
        let m = mk(&["^нет,"]);
        let r = match_markers("\"Нет, не так\"", &m).unwrap();
        assert_eq!(r.tier, "strong");
        let r = match_markers("ок", &m).unwrap();
        assert_eq!(r.role, "confirmation");
    }

    #[test]
    fn invalid_regex_rejected() {
        assert!(compile("(").is_err());
        let v = serde_json::json!({"version":1,"correction":{"strong":["("]}});
        assert_eq!(validate_markers(&v).len(), 1);
    }

    #[test]
    fn validate_shapes() {
        assert_eq!(
            validate_markers(&serde_json::json!([])),
            vec!["markers must be a JSON object"]
        );
        let e = validate_markers(&serde_json::json!({"version":2}));
        assert_eq!(e[0], "unsupported version: 2 (expected 1)");
        assert_eq!(e[1], "missing 'correction' object");
    }
}
