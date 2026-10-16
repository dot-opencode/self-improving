//! `learn dream`: report, print-config, stubbed OpenCode runs, locking.
mod common;
use common::*;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};

const STUB: &str = r#"#!/usr/bin/env bash
# stub: record the invocation, touch a candidate, print a summary line.
{
  printf 'cwd=%s\n' "$(pwd -P)"
  printf 'home=%s\n' "$AGENT_LEARNINGS_HOME"
  printf 'learn_dream=%s\n' "${LEARN_DREAM:-}"
  printf 'args=%s\n' "$*"
  printf 'config=%s\n' "$OPENCODE_CONFIG_CONTENT"
  printf 'db=%s\n' "$OPENCODE_DB"
  if [ -e "$OPENCODE_DB" ] || [ -e "$OPENCODE_DB-wal" ] || [ -e "$OPENCODE_DB-shm" ]; then
    echo "db-existed=yes"
  else
    echo "db-existed=no"
  fi
} > "$STUB_LOG"
printf '\nstub touched\n' >> "$AGENT_LEARNINGS_HOME/work/candidates/git.commit-scope.md"
mkdir -p "$AGENT_LEARNINGS_HOME/.drafts"
echo "leftover" > "$AGENT_LEARNINGS_HOME/.drafts/draft.md"
if [ "${STUB_INBOX:-0}" = 1 ]; then
  mkdir -p "$AGENT_LEARNINGS_HOME/work/inbox"
  printf '{"ts":"2026-09-30T23:00:00Z","kind":"failure","tool":"bash","head":"during-dream","project":"proj-a","session":"s9"}\n' \
    >> "$AGENT_LEARNINGS_HOME/work/inbox/$(date +%Y-%m-%d).jsonl"
fi
if [ "${STUB_BACKUP:-0}" = 1 ]; then
  {
    echo "head_before=$(git -C "$AGENT_LEARNINGS_HOME" rev-parse HEAD)"
    (cd "$STUB_BACKUP_CWD" && "$LEARN_FOR_STUB" backup 2>&1)
    echo "head_after=$(git -C "$AGENT_LEARNINGS_HOME" rev-parse HEAD)"
  } > "$STUB_BACKUP_LOG" 2>&1
fi
[ "${STUB_FAIL:-0}" = 1 ] && { echo "boom" >&2; exit 3; }
echo "working…"
echo "triaged 2 events → +1 candidates, ~0 bumped, 1 noise / 0 merged / 0 proposed"
"#;

const SUMMARY: &str =
    "triaged 2 events → +1 candidates, ~0 bumped, 1 noise / 0 merged / 0 proposed";

struct Fx {
    e: Env,
    dstore: PathBuf,
    dpdir: PathBuf,
    stubbin: PathBuf,
    stub_log: PathBuf,
}

fn dcand(f: &Fx, key: &str, desc: &str, seen: u32, last: &str, projects: &str, status: &str) {
    wr(
        f.dpdir.join(format!("candidates/{key}.md")),
        &format!(
            "---\ndescription: {desc}\npattern-key: {key}\nkind: correction\nseen: {seen}\nfirst-seen: 2025-01-01\nlast-seen: {last}\nprojects: [{projects}]\nstatus: {status}\n---\n## Future behavior\nDo the right thing.\n\n## Evidence\n- {last} proj-a: something happened\n"
        ),
    );
}

/// The shared dream fixture store: two similar candidates, one ready (seen=3),
/// one stale, two unacked inbox events. cwd is a neutral dir (no .learnings).
fn fixture() -> Fx {
    let mut e = Env::new();
    let dstore = e.root.join("dstore");
    let dpdir = dstore.join("work");
    let dcwd = e.root.join("dcwd");
    mkdirs(&dcwd);
    e.proj = dcwd;
    e.set("LEARN_TODAY", "2026-09-30");
    e.set("AGENT_LEARNINGS_HOME", dstore.to_str().unwrap());
    bare_store(&e, &dstore, "work");
    let stubbin = e.root.join("stubbin");
    mkdirs(&stubbin);
    let sp = stubbin.join("opencode");
    fs::write(&sp, STUB).unwrap();
    fs::set_permissions(&sp, fs::Permissions::from_mode(0o755)).unwrap();
    let stub_log = e.root.join("stub.log");
    e.set("STUB_LOG", stub_log.to_str().unwrap());
    let f = Fx {
        e,
        dstore,
        dpdir,
        stubbin,
        stub_log,
    };
    dcand(
        &f,
        "test.pytest-wrong-cwd",
        "run pytest from the repo root in a monorepo",
        1,
        "2026-09-29",
        "proj-a",
        "candidate",
    );
    dcand(
        &f,
        "test.pytest-cwd-root",
        "pytest monorepo: repo root cwd",
        1,
        "2026-09-29",
        "proj-a",
        "candidate",
    );
    dcand(
        &f,
        "git.commit-scope",
        "commit messages need a scope",
        3,
        "2026-09-29",
        "proj-a",
        "candidate",
    );
    dcand(
        &f,
        "env.old-thing",
        "some very old one-off",
        1,
        "2025-01-01",
        "proj-a",
        "candidate",
    );
    wr(
        f.dpdir.join("inbox/2026-09-29.jsonl"),
        "{\"ts\":\"2026-09-29T10:00:00Z\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"SECRET-EVENT-TEXT\",\"project\":\"proj-a\",\"session\":\"s1\"}\n\
{\"ts\":\"2026-09-29T11:00:00Z\",\"kind\":\"correction\",\"tier\":\"strong\",\"marker\":\"m\",\"snippet\":\"SECRET-EVENT-TEXT-2\",\"project\":\"proj-a\",\"session\":\"s1\"}\n",
    );
    f.e.run(&["index", "--profile", "work"]);
    f
}

impl Fx {
    fn stub(&self, args: &[&str]) -> Run {
        self.e
            .cmd(args)
            .env("PATH", self.e.path_with(&self.stubbin))
    }
    fn dirty(&self) -> String {
        self.e.git_out(
            &self.dstore,
            &["status", "--porcelain", "--untracked-files=all"],
        )
    }
    fn head(&self) -> String {
        self.e.git_out(&self.dstore, &["rev-parse", "HEAD"])
    }
    /// One successful stub dream so the store has history and a `.dream-last`.
    fn first_dream(&self) {
        self.stub(&["dream"]).run().ok("first dream");
    }
}

#[test]
fn report_sections() {
    let f = fixture();
    let e = &f.e;
    let o = e.run(&["dream", "--report"]);
    o.has("dream --report: unacked section", "unacked inbox")
        .has(
            "dream --report: unacked count and range",
            "[work] 2 event(s), oldest 2026-09-29T10:00:00Z, newest 2026-09-29T11:00:00Z",
        )
        .lacks("dream --report: no event text", "SECRET-EVENT-TEXT")
        .has("dream --report: similar section", "similar candidates")
        .has(
            "dream --report: similar pair",
            "[work] candidates/test.pytest-cwd-root.md <-> candidates/test.pytest-wrong-cwd.md",
        )
        .has("dream --report: ready section", "ready for proposal")
        .has(
            "dream --report: ready item",
            "[work] git.commit-scope — seen 3",
        )
        .has(
            "dream --report: stale section",
            "stale (seen 1, older than 90 days",
        )
        .has("dream --report: stale item", "[work] env.old-thing")
        .has(
            "dream --report: summary",
            "dream: 2 unacked, 1 similar, 1 ready, 1 stale, 0 unusable",
        );

    // proposal section removes it from "ready"; ack clears "unacked"
    let cs = f.dpdir.join("candidates/git.commit-scope.md");
    wr(&cs, &(rd(&cs) + "\n## Proposal\n- **Owner:** script:x\n"));
    e.run(&[
        "ack",
        "--profile",
        "work",
        "--until",
        "2026-09-29T11:00:00Z",
    ]);
    e.run(&["dream", "--report", "--profile", "work"])
        .lacks(
            "dream --report: candidate with Proposal not ready",
            "git.commit-scope — seen",
        )
        .has(
            "dream --report: acked inbox gone from summary",
            "dream: 0 unacked, 1 similar, 0 ready, 1 stale, 0 unusable",
        );

    // proposed without decision (>14d), unusable and hygiene sections
    dcand(
        &f,
        "build.proposed-long",
        "waiting for the user",
        3,
        "2026-09-01",
        "proj-a",
        "proposed",
    );
    wr(f.dpdir.join("candidates/git.broken.md"), "no frontmatter\n");
    dcand(
        &f,
        "zzz.odd-area",
        "unknown area here",
        1,
        "2026-09-29",
        "proj-a",
        "candidate",
    );
    e.run(&["index", "--profile", "work"]);
    e.run(&["dream", "--report"])
        .has(
            "dream --report: proposed-without-decision section",
            "proposed without decision",
        )
        .has(
            "dream --report: proposed item",
            "[work] build.proposed-long",
        )
        .has("dream --report: unusable section", "unusable")
        .has(
            "dream --report: unusable frontmatter",
            "[work] candidates/git.broken.md",
        )
        .has("dream --report: hygiene unknown area", "unknown area 'zzz'");
    let o = e.run(&["dream", "--report", "--profile", "nope"]);
    check(
        "dream --report unknown --profile dies",
        o.rc != 0 && o.out().contains("no such profile"),
    );
    e.run(&["dream", "--report", "--model", "a/b"])
        .die("dream --report rejects --model", "usage: learn dream");
    e.run(&["dream", "--stale-days", "5"]).die(
        "dream rejects --stale-days without --report",
        "usage: learn dream",
    );
    e.run(&["dream", "--report", "--stale-days", "5000"]).lacks(
        "dream --report --stale-days widens window",
        "[work] env.old-thing",
    );

    // --profile on ordinary commands (used by the dream agent from the store root)
    e.run(&["inbox", "--profile", "work", "--all"])
        .has("inbox --profile reads that profile", "SECRET-EVENT-TEXT");
    let o = e.run(&["doctor", "--profile", "work"]);
    check(
        "doctor --profile flags bad frontmatter",
        o.rc != 0 && o.out().contains("BAD FRONTMATTER"),
    );
    for k in ["git.broken", "zzz.odd-area", "build.proposed-long"] {
        fs::remove_file(f.dpdir.join(format!("candidates/{k}.md"))).unwrap();
    }
    e.run(&["index", "--profile", "work"]);
    e.run(&["doctor", "--profile", "work"])
        .ok("doctor --profile clean");
}

#[test]
fn report_tidy_store() {
    let e = Env::new();
    let t = e.root.join("tstore");
    bare_store(&e, &t, "home");
    let run = |args: &[&str]| {
        e.cmd(args)
            .env("AGENT_LEARNINGS_HOME", &t)
            .env("LEARN_TODAY", "2026-09-30")
            .run()
    };
    run(&[
        "add",
        "--profile",
        "home",
        "--key",
        "tool.only-one",
        "--kind",
        "workflow",
        "--summary",
        "the only candidate",
        "--evidence",
        "e",
        "--project",
        "proj-a",
    ]);
    run(&[
        "ack",
        "--profile",
        "home",
        "--until",
        "2026-09-30T00:00:00Z",
    ]);
    run(&["dream", "--report"]).has(
        "dream --report tidy store",
        "dream: learnings are tidy — nothing to consolidate",
    );
}

#[test]
fn print_config_and_glob_guard() {
    let f = fixture();
    let e = &f.e;
    let o = e.run(&["dream", "--print-config"]);
    o.ok("dream --print-config exits 0")
        .lacks("print-config: no __STORE__", "__STORE__")
        .lacks("print-config: no __LEARN_BIN__", "__LEARN_BIN__")
        .lacks("print-config: no __HOME__", "__HOME__")
        .has(
            "print-config: store substituted",
            &format!("edit:{}/*", f.dstore.display()),
        )
        .has(
            "print-config: learn bin substituted",
            &format!("shell:{} inbox*", LEARN),
        )
        .has(
            "print-config: home substituted",
            &format!("read:{}/.ssh/*", e.home.display()),
        )
        .has("print-config: learn-dream agent", "\"learn-dream\"");
    check(
        "print-config is valid JSON",
        serde_json::from_str::<serde_json::Value>(&o.stdout).is_ok(),
    );

    for gc in ["*", "?", "[x]", "{a,b}"] {
        let home = e.root.join(format!("gl{gc}ob"));
        let o = e.cmd(&["dream", "--print-config"]).env("HOME", &home).run();
        check(
            &format!("dream rejects glob char '{gc}' in HOME"),
            o.rc != 0 && o.out().contains("path contains a glob character"),
        );
    }
}

#[test]
fn dream_without_opencode() {
    let f = fixture();
    let empty = f.e.root.join("emptybin");
    mkdirs(&empty);
    let path = format!("{}:/usr/bin:/bin", empty.display());
    let has_oc = ["/usr/bin/opencode", "/bin/opencode"]
        .iter()
        .any(|p| std::path::Path::new(p).exists());
    if has_oc {
        eprintln!("SKIP: opencode exists in /usr/bin or /bin");
        return;
    }
    let o = f.e.cmd(&["dream"]).env("PATH", path).run();
    check(
        "dream without opencode dies cleanly",
        o.rc != 0 && o.out().contains("opencode not found on PATH"),
    );
}

#[test]
fn stub_run_commits_and_if_changed() {
    let f = fixture();
    let e = &f.e;
    e.run(&["index", "--profile", "work"]); // store is dirty (uncommitted fixture) -> pre-dream snapshot
                                            // --report / --print-config must not touch an existing dream DB
    let db = f.dstore.join(".git/learn-dream.db");
    wr(&db, "stale\n");
    wr(f.dstore.join(".git/learn-dream.db-wal"), "stale\n");
    e.run(&["dream", "--report"]);
    check("dream --report keeps .git/learn-dream.db", db.exists());
    e.run(&["dream", "--print-config"]);
    check(
        "dream --print-config keeps .git/learn-dream.db",
        db.exists(),
    );

    let o = f.stub(&["dream", "--model", "prov/model-x"]).run();
    o.ok("dream with stub opencode exits 0");
    let log = rd(&f.stub_log);
    let has_line = |l: &str| log.lines().any(|x| x == l);
    check(
        "stub got OPENCODE_DB in store .git",
        has_line(&format!("db={}", db.display())),
    );
    check(
        "stale dream db removed before the run",
        has_line("db-existed=no"),
    );
    o.has(
        "dream prints session transcript location",
        "session transcript in",
    )
    .has(
        "dream prints undo hint",
        &format!("undo: git -C {} revert HEAD", f.dstore.display()),
    );
    check(
        "dream db never shows in git status",
        !f.dirty().contains("learn-dream.db"),
    );
    let subjects = e.git_out(&f.dstore, &["log", "--format=%s"]);
    check(
        "dream made pre-dream snapshot commit",
        subjects.contains("learn: pre-dream snapshot"),
    );
    eq::<String, _>(
        "dream commit message uses agent summary",
        e.git_out(&f.dstore, &["log", "-1", "--format=%s"]),
        format!("learn: dream — {SUMMARY}"),
    );
    check("dream leaves store clean", f.dirty().is_empty());
    check(
        "dream: .drafts/ not in git status",
        !f.dirty().contains(".drafts"),
    );
    check(
        "dream: leftover draft removed",
        !f.dstore.join(".drafts/draft.md").exists(),
    );
    check(
        "dream: .drafts/ not committed",
        e.git_out(&f.dstore, &["ls-files", ".drafts"]).is_empty(),
    );
    check(
        "store exclude lists .drafts/",
        rd(f.dstore.join(".git/info/exclude"))
            .lines()
            .any(|l| l == "/.drafts/"),
    );
    eq::<String, _>(
        ".dream-last == HEAD",
        rd(f.dstore.join(".dream-last")).trim().to_string(),
        f.head(),
    );
    check(
        "stub ran in store root",
        has_line(&format!("cwd={}", f.dstore.display())),
    );
    check(
        "stub got pinned AGENT_LEARNINGS_HOME",
        has_line(&format!("home={}", f.dstore.display())),
    );
    check("stub got LEARN_DREAM=1", has_line("learn_dream=1"));
    check(
        "stub got --standalone --agent learn-dream --model",
        log.lines().any(|l| {
            l.starts_with("args=run --standalone --agent learn-dream --model prov/model-x ")
        }),
    );
    check(
        "stub config has no placeholders",
        !log.contains("__STORE__") && !log.contains("__LEARN_BIN__") && !log.contains("__HOME__"),
    );
    check(
        "prompt reached the stub",
        log.contains("Goal: consolidate the store"),
    );

    // --if-changed: skip when nothing changed since .dream-last
    let head_before = f.head();
    fs::remove_file(&f.stub_log).unwrap();
    let o = f.stub(&["dream", "--if-changed"]).run();
    check(
        "dream --if-changed skips when .dream-last == HEAD",
        o.rc == 0 && o.out().contains("skipping") && !f.stub_log.exists(),
    );
    eq::<String, _>("skip made no commit", f.head(), head_before);
    wr(f.dstore.join("notes.txt"), "x\n");
    let o = f.stub(&["dream", "--if-changed"]).run();
    check(
        "dream --if-changed ignores unrelated files",
        o.rc == 0 && !f.stub_log.exists(),
    );
    wr(
        f.dpdir.join("inbox/2026-09-30.jsonl"),
        "{\"ts\":\"2026-09-30T08:00:00Z\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"x\",\"project\":\"proj-a\",\"session\":\"s2\"}\n",
    );
    let o = f.stub(&["dream", "--if-changed"]).run();
    check(
        "dream --if-changed runs after inbox change",
        o.rc == 0 && f.stub_log.exists(),
    );
    eq::<String, _>(
        ".dream-last advanced to HEAD",
        rd(f.dstore.join(".dream-last")).trim().to_string(),
        f.head(),
    );

    // failing opencode: die, leave changes uncommitted, keep .dream-last
    let last_sha = rd(f.dstore.join(".dream-last")).trim().to_string();
    let o = f.stub(&["dream"]).env("STUB_FAIL", "1").run();
    check(
        "dream dies when opencode fails",
        o.rc != 0 && o.out().contains("left uncommitted for review"),
    );
    check(
        "failed dream keeps changes uncommitted",
        !f.dirty().is_empty(),
    );
    eq::<String, _>(
        "failed dream keeps .dream-last",
        rd(f.dstore.join(".dream-last")).trim().to_string(),
        last_sha,
    );

    // dream creates .drafts/ before the agent starts
    fs::remove_dir_all(f.dstore.join(".drafts")).ok();
    // a failing stub that still recreates .drafts would hide the check: remove it first, stub fails before? it creates it,
    // so instead verify via a stub that records existence at start.
    let probe = f.e.root.join("probebin");
    mkdirs(&probe);
    let pp = probe.join("opencode");
    fs::write(&pp, "#!/usr/bin/env bash\n[ -d \"$AGENT_LEARNINGS_HOME/.drafts\" ] && echo yes > \"$PROBE_OUT\" || echo no > \"$PROBE_OUT\"\nexit 3\n").unwrap();
    fs::set_permissions(&pp, fs::Permissions::from_mode(0o755)).unwrap();
    let pout = f.e.root.join("probe.out");
    fs::remove_dir_all(f.dstore.join(".drafts")).ok();
    f.e.cmd(&["dream"])
        .env("PATH", f.e.path_with(&probe))
        .env("PROBE_OUT", &pout)
        .run();
    check(
        "dream pre-creates .drafts/ before the agent runs",
        rd(&pout).trim() == "yes" && f.dstore.join(".drafts").is_dir(),
    );
    check(
        "failed dream (STUB_FAIL) leaves no .dream.lock",
        !f.dstore.join(".dream.lock").exists(),
    );
}

fn sleeper() -> std::process::Child {
    Command::new("sleep")
        .arg("30")
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn lock_live_and_stale() {
    let f = fixture();
    let e = &f.e;
    f.first_dream();
    wr(f.dpdir.join("inbox/2026-09-30.jsonl"), "{\"ts\":\"2026-09-30T08:00:00Z\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"x\",\"project\":\"p\"}\n");
    let lock = f.dstore.join(".dream.lock");
    check(
        "store exclude lists .dream.lock",
        rd(f.dstore.join(".git/info/exclude"))
            .lines()
            .any(|l| l == "/.dream.lock"),
    );
    let mut live = sleeper();
    let live_pid = live.id().to_string();
    wr(&lock, &format!("{live_pid}\n"));
    let head_locked = f.head();
    check(
        "store is dirty before the live-lock checks",
        !f.dirty().is_empty(),
    );
    let o = e.run(&["backup"]);
    o.has("backup with live dream lock is skipped", "backup skipped")
        .has(
            "backup skip names the dream pid",
            &format!("pid {live_pid}"),
        );
    eq::<i32, _>("backup skip exits 0", o.rc, 0);
    eq::<String, _>(
        "backup with live lock makes no commit",
        f.head(),
        head_locked.clone(),
    );
    check(
        "backup with live lock stages nothing",
        e.git_out(&f.dstore, &["diff", "--cached", "--name-only"])
            .is_empty(),
    );
    e.run(&["status", "--profile", "work"]).has(
        "status shows running dream",
        &format!("running (pid {live_pid})"),
    );
    let _ = fs::remove_file(&f.stub_log);
    let o = f.stub(&["dream"]).run();
    check(
        "dream dies while another dream holds the lock",
        o.rc != 0
            && o.out()
                .contains(&format!("another learn dream is running (pid {live_pid})")),
    );
    check("refused dream did not run the agent", !f.stub_log.exists());
    eq::<String, _>(
        "refused dream leaves the other lock alone",
        rd(&lock).trim().to_string(),
        live_pid,
    );
    live.kill().ok();
    live.wait().ok();

    // stale lock (dead pid): backup proceeds, dream replaces it and cleans up
    let mut dead = Command::new("true").spawn().unwrap();
    dead.wait().unwrap();
    let dead_pid = dead.id().to_string();
    wr(&lock, &format!("{dead_pid}\n"));
    e.run(&["status", "--profile", "work"])
        .lacks("status ignores a stale lock", "running (pid");
    e.run(&["backup"])
        .has("backup with stale lock commits", "backed up:");
    check("backup with stale lock moved HEAD", f.head() != head_locked);
    wr(&lock, &format!("{dead_pid}\n"));
    let o = f.stub(&["dream"]).run();
    check(
        "dream replaces a stale lock",
        o.rc == 0 && f.stub_log.exists(),
    );
    check("no .dream.lock after successful dream", !lock.exists());
    wr(&lock, "not a pid\n");
    let o = f.stub(&["dream"]).run();
    check("dream replaces an unreadable lock", o.rc == 0);
    check(
        "no .dream.lock after dream over garbage lock",
        !lock.exists(),
    );
    let o = f.stub(&["dream"]).env("STUB_FAIL", "1").run();
    check("STUB_FAIL dream still fails", o.rc != 0);
    check("no .dream.lock after STUB_FAIL dream", !lock.exists());
    // --if-changed skip path releases the lock too
    e.run(&["backup"]);
    f.stub(&["dream"]).run();
    let o = f.stub(&["dream", "--if-changed"]).run();
    check(
        "dream --if-changed skips (lock test)",
        o.out().contains("skipping"),
    );
    check("no .dream.lock after --if-changed skip", !lock.exists());
    // the lock is local bookkeeping: never tracked, never committed
    check(
        ".dream.lock not in git ls-files",
        !e.git_out(&f.dstore, &["ls-files"]).contains("dream.lock"),
    );
    check(
        ".dream.lock in no commit",
        !e.git_out(&f.dstore, &["log", "--all", "--name-only", "--format="])
            .contains("dream.lock"),
    );
    check("store clean after lock scenarios", f.dirty().is_empty());
}

#[test]
fn inbox_events_during_dream_get_own_commit() {
    let f = fixture();
    f.first_dream();
    let o = f.stub(&["dream"]).env("STUB_INBOX", "1").run();
    o.ok("dream with STUB_INBOX exits 0");
    let subj2 = f.e.git_out(&f.dstore, &["log", "--format=%s", "-2"]);
    eq::<String, _>(
        "dream commit follows a separate inbox-events commit",
        subj2,
        format!("learn: dream — {SUMMARY}\nlearn: inbox events recorded during dream"),
    );
    let show = |rev: &str| {
        f.e.git_out(&f.dstore, &["show", "--name-only", "--format=", rev])
    };
    let is_inbox = |s: &str| {
        s.lines()
            .any(|l| l.contains("inbox/") && l.ends_with(".jsonl"))
    };
    check(
        "inbox-events commit holds the jsonl",
        is_inbox(&show("HEAD~1")),
    );
    check(
        "dream commit holds no inbox jsonl",
        !is_inbox(&show("HEAD")),
    );
    check(
        "dream commit holds the agent's changes",
        show("HEAD").contains("candidates/"),
    );
    check("store clean after STUB_INBOX dream", f.dirty().is_empty());
    check(
        "no .dream.lock after STUB_INBOX dream",
        !f.dstore.join(".dream.lock").exists(),
    );
}

#[test]
fn backup_mid_dream_is_skipped() {
    let f = fixture();
    f.first_dream();
    let bproj = f.e.root.join("bproj");
    mkdirs(&bproj);
    symlink(&f.dpdir, bproj.join(".learnings")).unwrap();
    let blog = f.e.root.join("stub-backup.log");
    let o = f
        .stub(&["dream"])
        .env("STUB_BACKUP", "1")
        .env("STUB_BACKUP_CWD", &bproj)
        .env("STUB_BACKUP_LOG", &blog)
        .env("LEARN_FOR_STUB", LEARN)
        .run();
    o.ok("dream with mid-dream backup exits 0");
    let log = rd(&blog);
    check(
        "mid-dream backup was skipped",
        log.contains("backup skipped"),
    );
    let get = |k: &str| {
        log.lines()
            .find_map(|l| l.strip_prefix(k))
            .unwrap_or("")
            .to_string()
    };
    check(
        "mid-dream backup made no commit",
        get("head_before=") == get("head_after=") && !get("head_before=").is_empty(),
    );
    let subj = f.e.git_out(&f.dstore, &["log", "-1", "--format=%s"]);
    check(
        "dream commit is HEAD after mid-dream backup",
        subj.starts_with("learn: dream "),
    );
    check(
        "dream commit holds the agent's changes (mid-dream backup)",
        f.e.git_out(&f.dstore, &["show", "--name-only", "--format=", "HEAD"])
            .contains("candidates/"),
    );
    check(
        "no .dream.lock after mid-dream backup dream",
        !f.dstore.join(".dream.lock").exists(),
    );
}
