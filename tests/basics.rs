//! Basics and `init` (run.sh sections: basics, init).
mod common;
use common::*;
use std::fs;

#[test]
fn version_help_unknown() {
    let e = Env::new();
    let o = e.run(&["--version"]);
    check("--version", o.rc == 0 && o.stdout.starts_with("learn "));
    eq::<String, _>(
        "--version is learn 0.1.2",
        o.stdout.trim().to_string(),
        "learn 0.1.2",
    );
    e.run(&["help"]).ok("help");
    e.run(&["nope"])
        .die("unknown command dies", "unknown command");
}

#[test]
fn init_creates_store() {
    let e = Env::new();
    e.run(&["init"])
        .die("init without profile lists profiles", "usage: learn init");
    e.run(&["init", "work"]).ok("init work");
    let p = e.pdir();
    check("store is git repo", is_dir(e.store.join(".git")));
    check(
        "profile dirs created",
        ["inbox", "candidates", "applied", "rejected"]
            .iter()
            .all(|d| is_dir(p.join(d))),
    );
    check("INDEX.md created", p.join("INDEX.md").is_file());
    check(
        "markers.json created",
        e.store.join("markers.json").is_file(),
    );
    check(
        "markers.json is valid JSON",
        serde_json::from_str::<serde_json::Value>(&rd(e.store.join("markers.json"))).is_ok(),
    );
    eq::<std::path::PathBuf, _>(
        ".learnings symlink -> profile",
        fs::read_link(e.proj.join(".learnings")).unwrap(),
        p.clone(),
    );
    let ignore = e.home.join(".config/git/ignore");
    check(
        "global gitignore has .learnings (no trailing slash)",
        rd(&ignore).lines().any(|l| l == ".learnings"),
    );
    let (rc, _) = e.git(&e.proj, &["init", "-q", "."]);
    eq::<i32, _>("git init proj", rc, 0);
    let (rc, _) = e.git(&e.proj, &["check-ignore", "-q", ".learnings"]);
    eq::<i32, _>("git ignores the .learnings symlink", rc, 0);
    let (_, st) = e.git(&e.proj, &["status", "--porcelain"]);
    check("git status hides .learnings", !st.contains(".learnings"));
}

#[test]
fn init_repairs_legacy_gitignore_and_is_idempotent() {
    let e = Env::new();
    e.init_work();
    let ignore = e.home.join(".config/git/ignore");
    wr(
        &ignore,
        "**/.claude/settings.local.json\n.memory/\n.learnings/\n",
    );
    e.run(&["init", "work"])
        .ok("init repairs legacy .learnings/ pattern")
        .has("init reports the gitignore fix", "fixed global gitignore");
    let t = rd(&ignore);
    check(
        "legacy pattern replaced",
        t.lines().filter(|l| *l == ".learnings").count() == 1
            && t.lines().filter(|l| *l == ".learnings/").count() == 0,
    );
    check(
        "other gitignore lines kept",
        t.lines().any(|l| l == ".memory/"),
    );
    e.run(&["init", "work"]).ok("init is idempotent");
    e.run(&["init", "Bad Name"])
        .die("init rejects bad profile", "invalid profile");
    e.run(&["init", "work", "--name", "foo"])
        .die("init no longer accepts --name", "unknown option");
}
