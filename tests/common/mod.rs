//! Shared helpers for the integration tests: an isolated environment per test
//! (own HOME / store / project dir / git config) and a tiny runner for the
//! `learn` binary. Every test builds its own `Env`, so tests are parallel-safe.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub const LEARN: &str = env!("CARGO_BIN_EXE_learn");

pub struct Env {
    pub tmp: TempDir,
    pub root: PathBuf,
    pub home: PathBuf,
    pub store: PathBuf,
    pub proj: PathBuf,
    envs: Vec<(String, Option<String>)>,
}

pub struct Out {
    pub rc: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct Run {
    cmd: Command,
}

impl Env {
    pub fn new() -> Env {
        let tmp = tempfile::Builder::new()
            .prefix("learn-test.")
            .tempdir()
            .expect("tempdir");
        let root = fs::canonicalize(tmp.path()).unwrap();
        let home = root.join("home");
        let store = root.join("store");
        let proj = root.join("proj");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&proj).unwrap();
        let e = Env {
            tmp,
            root,
            home,
            store,
            proj,
            envs: Vec::new(),
        };
        for (k, v) in [
            ("user.name", "learn test"),
            ("user.email", "learn@example.invalid"),
            ("init.defaultBranch", "main"),
        ] {
            let (rc, _) = e.git(&e.root, &["config", "--global", k, v]);
            assert_eq!(rc, 0, "git config --global {k}");
        }
        e
    }

    /// Default env override for every later command of this test.
    pub fn set(&mut self, k: &str, v: impl AsRef<str>) {
        self.envs.retain(|(n, _)| n != k);
        self.envs
            .push((k.to_string(), Some(v.as_ref().to_string())));
    }

    pub fn unset(&mut self, k: &str) {
        self.envs.retain(|(n, _)| n != k);
        self.envs.push((k.to_string(), None));
    }

    fn apply(&self, c: &mut Command) {
        for k in [
            "LEARN_TODAY",
            "LEARN_DREAM",
            "AGENT_LEARNINGS_DREAM_MODEL",
            "LEARN_SKILL_DIR",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
        ] {
            c.env_remove(k);
        }
        c.env("HOME", &self.home)
            .env("AGENT_LEARNINGS_HOME", &self.store)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LEARN_PROJECT", "proj-a");
        for (k, v) in &self.envs {
            match v {
                Some(v) => c.env(k, v),
                None => c.env_remove(k),
            };
        }
    }

    pub fn cmd(&self, args: &[&str]) -> Run {
        let mut c = Command::new(LEARN);
        c.args(args).current_dir(&self.proj);
        self.apply(&mut c);
        Run { cmd: c }
    }

    pub fn run(&self, args: &[&str]) -> Out {
        self.cmd(args).run()
    }

    /// Run git in `dir` with the isolated environment; returns (rc, stdout trimmed).
    pub fn git(&self, dir: &Path, args: &[&str]) -> (i32, String) {
        let mut c = Command::new("git");
        c.args(args).current_dir(dir);
        self.apply(&mut c);
        let o = c.output().expect("git");
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).trim_end().to_string(),
        )
    }

    pub fn git_out(&self, dir: &Path, args: &[&str]) -> String {
        self.git(dir, args).1
    }

    pub fn pdir(&self) -> PathBuf {
        self.store.join("work")
    }

    pub fn init_work(&self) {
        self.run(&["init", "work"]).ok("init work");
    }

    /// Environment value as seen by commands (for building PATH etc).
    pub fn path_with(&self, first: &Path) -> String {
        format!(
            "{}:{}",
            first.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }
}

impl Run {
    pub fn env(mut self, k: &str, v: impl AsRef<std::ffi::OsStr>) -> Run {
        self.cmd.env(k, v);
        self
    }

    pub fn unset(mut self, k: &str) -> Run {
        self.cmd.env_remove(k);
        self
    }

    pub fn cwd(mut self, p: &Path) -> Run {
        self.cmd.current_dir(p);
        self
    }

    pub fn run(mut self) -> Out {
        let o = self.cmd.output().expect("run learn");
        Out {
            rc: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }
}

impl Out {
    /// stdout followed by stderr (the bash tests capture `2>&1`).
    pub fn out(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }

    pub fn ok(&self, name: &str) -> &Out {
        assert_eq!(self.rc, 0, "{name}: exit {}: {}", self.rc, self.out());
        self
    }

    pub fn die(&self, name: &str, sub: &str) -> &Out {
        assert!(
            self.rc != 0 && self.out().contains(sub),
            "{name}: exit {} (expected failure containing {sub:?}): {}",
            self.rc,
            self.out()
        );
        self
    }

    pub fn has(&self, name: &str, needle: &str) -> &Out {
        assert!(
            self.out().contains(needle),
            "{name}: missing {needle:?} in: {}",
            self.out()
        );
        self
    }

    pub fn lacks(&self, name: &str, needle: &str) -> &Out {
        assert!(
            !self.out().contains(needle),
            "{name}: unexpected {needle:?} in: {}",
            self.out()
        );
        self
    }

    /// Number of output lines starting with `prefix`.
    pub fn count(&self, prefix: &str) -> usize {
        self.out().lines().filter(|l| l.starts_with(prefix)).count()
    }
}

pub fn check(name: &str, cond: bool) {
    assert!(cond, "{name}");
}

pub fn eq<A: PartialEq + std::fmt::Debug, B: Into<A>>(name: &str, left: A, right: B) {
    assert_eq!(left, right.into(), "{name}");
}

pub fn rd(p: impl AsRef<Path>) -> String {
    fs::read_to_string(p).unwrap_or_default()
}

pub fn wr(p: impl AsRef<Path>, s: &str) {
    if let Some(d) = p.as_ref().parent() {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(p, s).unwrap();
}

pub fn exists(p: impl AsRef<Path>) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// Value of one flat frontmatter key (first `---` block only).
pub fn fmval(p: impl AsRef<Path>, key: &str) -> String {
    let text = rd(p);
    let prefix = format!("{key}: ");
    for (i, l) in text.lines().enumerate() {
        if i == 0 {
            continue;
        }
        if l == "---" {
            break;
        }
        if let Some(r) = l.strip_prefix(&prefix) {
            return r.to_string();
        }
    }
    String::new()
}

/// Number of lines of `p` that match the regex.
pub fn count_re(p: impl AsRef<Path>, re: &str) -> usize {
    let re = regex::Regex::new(re).unwrap();
    rd(p).lines().filter(|l| re.is_match(l)).count()
}

pub fn today_local() -> String {
    let o = Command::new("date").arg("+%Y-%m-%d").output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

pub fn is_dir(p: impl AsRef<Path>) -> bool {
    p.as_ref().is_dir()
}

pub fn lines_with(text: &str, prefix: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.starts_with(prefix))
        .map(String::from)
        .collect()
}

pub fn mkdirs(p: impl AsRef<Path>) {
    fs::create_dir_all(p).unwrap();
}

/// Store skeleton for `--profile` tests: `<root>/work/{inbox,candidates,applied,rejected}` + git init.
pub fn bare_store(e: &Env, dir: &Path, profile: &str) {
    for d in ["inbox", "candidates", "applied", "rejected"] {
        mkdirs(dir.join(profile).join(d));
    }
    let (rc, _) = e.git(dir, &["init", "-q"]);
    assert_eq!(rc, 0);
}
