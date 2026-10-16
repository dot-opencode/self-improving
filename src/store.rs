//! Store/profile resolution, frontmatter handling and INDEX.md rendering.

use crate::util::*;
use std::fs;
use std::path::Path;

pub const AREAS: &str =
    "test build deps git shell edit scope spec ui lang verbosity tool env infra";
pub const KINDS: &str = "correction failure confirmation workflow";

pub struct Ctx {
    pub learn_home: String,
    pub store_home: String,
    pub profile_override: String,
    pub profile: String,
    pub pdir: String,
    pub skill_dir: String,
    pub script_path: String,
    pub doc_problems: usize,
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

pub fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_default()
}

pub fn pwd_p() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| fs::canonicalize(p).ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Locate the skill directory (skills/self-improving): holds `opencode/`
/// (default markers/hints JSON, plugin) and `dream/` (OpenCode config+prompt).
///
/// Resolution order:
///  1. `LEARN_SKILL_DIR` (set by the release shim; honored first, unconditionally);
///  2. `<dir of the canonical executable>/..` when it looks like a skill dir
///     (contains SKILL.md or opencode/) — the layout `skills/self-improving/bin/learn`;
///  3. any ancestor `<a>/skills/self-improving` of the executable (dev builds in
///     `target/release/`);
///  4. fallback: `<exe dir>/..` regardless.
fn find_skill_dir(exe: &str) -> String {
    if let Some(d) = env_nonempty("LEARN_SKILL_DIR") {
        return canon_dir(&d).unwrap_or(d);
    }
    let looks = |d: &str| is_file(&format!("{d}/SKILL.md")) || is_dir(&format!("{d}/opencode"));
    let parent = dirname(&dirname(exe));
    let first = dirname(exe) + "/..";
    if let Some(c) = canon_dir(&first) {
        if looks(&c) {
            return c;
        }
    }
    let mut cur = dirname(exe);
    loop {
        let cand = format!("{cur}/skills/self-improving");
        if looks(&cand) {
            return canon_dir(&cand).unwrap_or(cand);
        }
        let up = dirname(&cur);
        if up == cur {
            break;
        }
        cur = up;
    }
    canon_dir(&first).unwrap_or(parent)
}

impl Ctx {
    pub fn new() -> Ctx {
        let mut lh = env_nonempty("AGENT_LEARNINGS_HOME")
            .unwrap_or_else(|| format!("{}/.agent-learnings", home_dir()));
        if !lh.starts_with('/') {
            lh = format!("{}/{}", pwd_p(), lh);
        }
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| fs::canonicalize(p).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| die("cannot resolve CLI path"));
        let skill_dir = find_skill_dir(&exe);
        Ctx {
            store_home: lh.clone(),
            learn_home: lh,
            profile_override: String::new(),
            profile: String::new(),
            pdir: String::new(),
            skill_dir,
            // The bootstrap shim exports its own canonical path: that is the
            // command agents invoke (e.g. in the dream sandbox), not the cached binary.
            script_path: env_nonempty("LEARN_CLI_PATH").unwrap_or(exe),
            doc_problems: 0,
        }
    }

    pub fn opencode_dir(&self) -> String {
        format!("{}/opencode", self.skill_dir)
    }

    pub fn is_in_store(&self, p: &str) -> bool {
        p == self.store_home || p.starts_with(&format!("{}/", self.store_home))
    }

    pub fn load_store(&mut self) {
        if !is_dir(&self.learn_home) {
            die(&format!(
                "no store at {} (ask the user to run: learn init <profile>)",
                self.learn_home
            ));
        }
        self.store_home = canon_dir(&self.learn_home).unwrap_or_else(|| self.learn_home.clone());
    }

    fn mk_profile_dirs(&self) {
        for d in ["inbox", "candidates", "applied", "rejected"] {
            mkdir_p(&format!("{}/{}", self.pdir, d));
        }
    }

    /// Sets profile and pdir (canonical profile dir) from `--profile` or ./.learnings.
    pub fn resolve_profile(&mut self) {
        if !self.profile_override.is_empty() {
            let po = self.profile_override.clone();
            require_slug("profile name", &po);
            self.load_store();
            self.profile = po.clone();
            let p = format!("{}/{}", self.store_home, po);
            if !is_dir(&p) {
                die(&format!("no such profile in the store: {po}"));
            }
            self.pdir = canon_dir(&p).unwrap_or(p);
            if !self.is_in_store(&self.pdir) {
                die(&format!(
                    "unsafe profile directory: {} is outside {}",
                    self.pdir, self.store_home
                ));
            }
            self.mk_profile_dirs();
            return;
        }
        if !is_symlink(".learnings") {
            die("no .learnings symlink here (ask the user to run: learn init <profile>)");
        }
        self.load_store();
        let target = fs::read_link(".learnings")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.profile = basename(&target);
        if !is_dir(".learnings") {
            die(&format!(
                "broken .learnings symlink -> {target} (ask the user to run: learn init <profile>)"
            ));
        }
        self.pdir = canon_dir(".learnings").unwrap_or_default();
        if !self.is_in_store(&self.pdir) {
            die(&format!(
                "unsafe .learnings symlink: {target} is outside {}",
                self.store_home
            ));
        }
        self.mk_profile_dirs();
    }

    pub fn read_ack(&self) -> String {
        first_line_of(Path::new(&format!("{}/inbox/.ack", self.pdir))).unwrap_or_default()
    }

    pub fn find_key(&self, key: &str) -> &'static str {
        for d in ["candidates", "applied", "rejected"] {
            if is_file(&format!("{}/{}/{}.md", self.pdir, d, key)) {
                return d;
            }
        }
        ""
    }

    pub fn validate_key(&self, key: &str) {
        if !valid_key(key) {
            die(&format!(
                "invalid key '{key}': use <area>.<symptom> (lowercase, digits, hyphens), e.g. test.wrong-cwd"
            ));
        }
        let area = key.split('.').next().unwrap_or("");
        if !in_list(area, AREAS) {
            warn(&format!(
                "warning: area '{area}' is not in the vocabulary ({AREAS})"
            ));
        }
    }

    // ---- INDEX.md -------------------------------------------------------

    /// INDEX.md content: one line per usable candidate, sorted by key (bytewise).
    pub fn render_index(&self, quiet: bool) -> String {
        let mut rows: Vec<(String, String)> = Vec::new();
        for f in glob_suffix(&format!("{}/candidates", self.pdir), ".md") {
            let base = basename(&f);
            let key = base.trim_end_matches(".md").to_string();
            let key = if base.ends_with(".md") {
                base[..base.len() - 3].to_string()
            } else {
                key
            };
            let problems = entry_problems(&f);
            if let Some(first) = problems.first() {
                if !quiet {
                    warn(&format!("skipping candidates/{base}: {first}"));
                }
                continue;
            }
            let kind = fm_get(&f, "kind");
            let seen = fm_get(&f, "seen");
            let desc = fm_get(&f, "description");
            rows.push((
                key.clone(),
                format!("- [{key}](candidates/{base}) — {kind} ×{seen}: {desc}\n"),
            ));
        }
        rows.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        rows.into_iter().map(|r| r.1).collect()
    }

    pub fn write_index(&self) {
        let content = self.render_index(false);
        write_atomic(
            &self.pdir,
            &format!("{}/INDEX.md", self.pdir),
            content.as_bytes(),
        );
    }
}

// ---- frontmatter --------------------------------------------------------

pub fn lines_of(text: &str) -> Vec<&str> {
    let mut v: Vec<&str> = text.split('\n').collect();
    if text.is_empty() || text.ends_with('\n') {
        v.pop();
    }
    v
}

/// Value of one flat frontmatter key; only the first ---...--- block is read.
pub fn fm_get(path: &str, key: &str) -> String {
    let Some(text) = read_text(path) else {
        return String::new();
    };
    let prefix = format!("{key}: ");
    for (i, line) in lines_of(&text).into_iter().enumerate() {
        if i == 0 {
            if line != "---" {
                return String::new();
            }
            continue;
        }
        if line == "---" {
            return String::new();
        }
        if let Some(rest) = line.strip_prefix(&prefix) {
            return rest.to_string();
        }
    }
    String::new()
}

/// Set (replace or append inside the block) one frontmatter key; atomic.
pub fn fm_set(path: &str, key: &str, val: &str) {
    let text = read_text(path).unwrap_or_default();
    let prefix = format!("{key}: ");
    let mut out = String::new();
    let mut state = 0;
    let mut done = false;
    let mut bad = false;
    for (i, line) in lines_of(&text).into_iter().enumerate() {
        if i == 0 {
            if line != "---" {
                bad = true;
            }
            state = 1;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if state == 1 && line == "---" {
            if !done {
                out.push_str(&format!("{key}: {val}\n"));
                done = true;
            }
            state = 2;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if state == 1 && line.starts_with(&prefix) {
            if !done {
                out.push_str(&format!("{key}: {val}\n"));
                done = true;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if bad || state != 2 {
        die(&format!("cannot update frontmatter of {path}"));
    }
    write_atomic(&dirname(path), path, out.as_bytes());
}

/// Append a line (creating "## Evidence" when absent); atomic.
pub fn append_evidence(path: &str, line: &str) {
    let mut text = fs::read(path).unwrap_or_default();
    if !text.is_empty() && text.last() != Some(&b'\n') {
        text.push(b'\n');
    }
    let has = String::from_utf8_lossy(&text)
        .split('\n')
        .any(|l| l == "## Evidence");
    if !has {
        text.extend_from_slice(b"\n## Evidence\n");
    }
    text.extend_from_slice(line.as_bytes());
    text.push(b'\n');
    write_atomic(&dirname(path), path, &text);
}

pub fn fm_has_block(path: &str) -> bool {
    let Some(text) = read_text(path) else {
        return false;
    };
    let lines = lines_of(&text);
    if lines.first() != Some(&"---") {
        return false;
    }
    lines[1..].contains(&"---")
}

/// Structural problems that make an entry unusable for INDEX.md.
pub fn entry_problems(f: &str) -> Vec<String> {
    let mut p = Vec::new();
    let base = basename(f);
    let stem = base.strip_suffix(".md").unwrap_or(&base).to_string();
    if !fm_has_block(f) {
        p.push(format!(
            "BAD FRONTMATTER (missing or unterminated ---): {f}"
        ));
        return p;
    }
    if fm_get(f, "description").is_empty() {
        p.push(format!("BAD FRONTMATTER (missing description): {f}"));
    }
    let key = fm_get(f, "pattern-key");
    if key != stem {
        p.push(format!(
            "BAD FRONTMATTER (pattern-key '{key}' does not match filename): {f}"
        ));
    }
    let v = fm_get(f, "kind");
    if !in_list(&v, KINDS) {
        p.push(format!(
            "BAD FRONTMATTER (missing or invalid kind '{v}'): {f}"
        ));
    }
    if !is_uint(&fm_get(f, "seen")) {
        p.push(format!("BAD FRONTMATTER (seen is not a number): {f}"));
    }
    if !valid_calendar_date(&fm_get(f, "first-seen")) {
        p.push(format!("BAD FRONTMATTER (invalid first-seen): {f}"));
    }
    if !valid_calendar_date(&fm_get(f, "last-seen")) {
        p.push(format!("BAD FRONTMATTER (invalid last-seen): {f}"));
    }
    p
}

/// Ensure local bookkeeping stays out of the store's commits.
pub fn ensure_store_excludes(store_home: &str) {
    let dir = format!("{store_home}/.git/info");
    mkdir_p(&dir);
    let ex = format!("{dir}/exclude");
    for p in ["/.dream-last", "/.drafts/", "/.dream.lock", ".learn.*"] {
        let mut cur = fs::read(&ex).unwrap_or_default();
        let has = String::from_utf8_lossy(&cur).split('\n').any(|l| l == p);
        if !has {
            if !cur.is_empty() && cur.last() != Some(&b'\n') {
                cur.push(b'\n');
            }
            cur.extend_from_slice(p.as_bytes());
            cur.push(b'\n');
            if fs::write(&ex, cur).is_err() {
                die(&format!("cannot write {ex}"));
            }
        }
    }
}
