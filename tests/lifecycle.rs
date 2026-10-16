//! add / bump / index / propose / apply / reject / drafts / dates.
mod common;
use common::*;
use std::fs;
use std::os::unix::fs::symlink;

fn add(e: &Env, key: &str, kind: &str, summary: &str, evidence: &str) -> Out {
    e.run(&[
        "add",
        "--key",
        key,
        "--kind",
        kind,
        "--summary",
        summary,
        "--evidence",
        evidence,
    ])
}

fn setup() -> Env {
    let e = Env::new();
    e.init_work();
    e
}

fn add_wrong_cwd(e: &Env) {
    e.run(&[
        "add",
        "--key",
        "test.wrong-cwd",
        "--kind",
        "correction",
        "--summary",
        "pytest в монорепо запускать из корня",
        "--evidence",
        "pytest tests/ -> ModuleNotFoundError conftest",
        "--behavior",
        "Run pytest from the repo root.",
    ])
    .ok("add test.wrong-cwd");
}

#[test]
fn add_validation_and_fields() {
    let e = setup();
    let p = e.pdir();
    add(&e, "BadKey", "correction", "s", "e").die("add rejects bad key", "invalid key");
    add(&e, "test.long", "correction", &"x".repeat(121), "e")
        .die("add rejects long summary", "max 120");
    add(&e, "test.badkind", "nonsense", "s", "e").die("add rejects bad kind", "invalid kind");

    add_wrong_cwd(&e);
    let f = p.join("candidates/test.wrong-cwd.md");
    check("candidate file created", f.is_file());
    eq::<String, _>("candidate seen: 1", fmval(&f, "seen"), "1");
    eq::<String, _>(
        "candidate status: candidate",
        fmval(&f, "status"),
        "candidate",
    );
    eq::<String, _>(
        "candidate projects: [proj-a]",
        fmval(&f, "projects"),
        "[proj-a]",
    );
    check(
        "candidate first-seen == last-seen",
        fmval(&f, "first-seen") == fmval(&f, "last-seen"),
    );
    check(
        "candidate has Future behavior",
        count_re(&f, "^## Future behavior$") == 1,
    );
    check(
        "candidate has behavior text",
        count_re(&f, r"^Run pytest from the repo root\.$") == 1,
    );
    check(
        "candidate has evidence line",
        count_re(&f, r"^- [0-9]{4}-[0-9]{2}-[0-9]{2} proj-a: pytest tests/") == 1,
    );
    add(&e, "test.wrong-cwd", "correction", "s", "e")
        .die("add duplicate key dies", "already exists");

    add(
        &e,
        "shell.no-sudo",
        "failure",
        "do not run sudo",
        "sudo: a password is required",
    )
    .ok("add shell.no-sudo (second)");
    let o = add(&e, "zzz.odd-area", "workflow", "unknown area warns", "e");
    check(
        "unknown area warns, does not die",
        o.rc == 0 && o.out().contains("not in the vocabulary"),
    );
    check(
        "behavior falls back to summary",
        count_re(p.join("candidates/shell.no-sudo.md"), "^do not run sudo$") == 1,
    );
}

#[test]
fn bump_and_index() {
    let e = setup();
    let p = e.pdir();
    add_wrong_cwd(&e);
    add(
        &e,
        "shell.no-sudo",
        "failure",
        "do not run sudo",
        "sudo: a password is required",
    )
    .ok("add");
    add(&e, "zzz.odd-area", "workflow", "unknown area warns", "e").ok("add");
    let f = p.join("candidates/test.wrong-cwd.md");

    e.run(&["bump", "test.wrong-cwd", "--evidence", "again wrong cwd"])
        .ok("bump (same project)");
    eq::<String, _>("bump seen -> 2", fmval(&f, "seen"), "2");
    eq::<String, _>(
        "bump keeps single project",
        fmval(&f, "projects"),
        "[proj-a]",
    );
    e.run(&[
        "bump",
        "test.wrong-cwd",
        "--evidence",
        "user: запускай из корня",
        "--project",
        "proj-b",
    ])
    .ok("bump with new project");
    eq::<String, _>("bump seen -> 3", fmval(&f, "seen"), "3");
    eq::<String, _>(
        "bump adds project",
        fmval(&f, "projects"),
        "[proj-a, proj-b]",
    );
    eq::<usize, _>(
        "bump appended evidence (3 lines)",
        count_re(&f, "^- [0-9]{4}-"),
        3usize,
    );
    check(
        "evidence for new project",
        rd(&f).contains("proj-b: user: запускай из корня"),
    );
    e.run(&["bump", "test.nothing", "--evidence", "x"])
        .die("bump unknown key dies", "no candidate");

    e.run(&["index"]).ok("index");
    let expected = "- [shell.no-sudo](candidates/shell.no-sudo.md) — failure ×1: do not run sudo\n\
- [test.wrong-cwd](candidates/test.wrong-cwd.md) — correction ×3: pytest в монорепо запускать из корня\n\
- [zzz.odd-area](candidates/zzz.odd-area.md) — workflow ×1: unknown area warns";
    eq::<String, _>(
        "INDEX.md matches expected lines",
        rd(p.join("INDEX.md")).trim_end().to_string(),
        expected,
    );

    // bad frontmatter is skipped with a warning
    wr(p.join("candidates/git.broken.md"), "no frontmatter here\n");
    let o = e.run(&["index"]);
    check(
        "index skips bad frontmatter with warning",
        o.rc == 0 && o.out().contains("skipping candidates/git.broken.md"),
    );
    check(
        "bad entry not in INDEX",
        !rd(p.join("INDEX.md")).contains("git.broken"),
    );
}

#[test]
fn propose_apply_reject() {
    let e = setup();
    let p = e.pdir();
    add_wrong_cwd(&e);
    add(
        &e,
        "shell.no-sudo",
        "failure",
        "do not run sudo",
        "sudo: a password is required",
    )
    .ok("add");
    let f = p.join("candidates/test.wrong-cwd.md");
    for n in ["a", "b"] {
        e.run(&["bump", "test.wrong-cwd", "--evidence", n])
            .ok("bump");
    }

    e.run(&["propose", "test.wrong-cwd", "--owner", "skill:pytest"])
        .ok("propose");
    eq::<String, _>("propose status: proposed", fmval(&f, "status"), "proposed");
    eq::<String, _>(
        "propose suggested-owner",
        fmval(&f, "suggested-owner"),
        "skill:pytest",
    );

    e.run(&[
        "apply",
        "test.wrong-cwd",
        "--owner",
        "nonsense",
        "--change-ref",
        "abc123",
    ])
    .die("apply rejects bad owner", "invalid owner");
    e.run(&[
        "add",
        "--key",
        "tool.owner-nl",
        "--kind",
        "workflow",
        "--summary",
        "owner flatten",
        "--evidence",
        "e",
        "--owner",
        "skill:a\nb",
    ])
    .ok("add with newline in owner");
    eq::<String, _>(
        "add owner newline flattened",
        fmval(p.join("candidates/tool.owner-nl.md"), "suggested-owner"),
        "skill:a b",
    );
    e.run(&[
        "apply",
        "tool.owner-nl",
        "--owner",
        "new-skill:foo\nbar",
        "--change-ref",
        "r1",
    ])
    .ok("apply new-skill owner (flattened)");
    eq::<String, _>(
        "apply owner newline flattened",
        fmval(p.join("applied/tool.owner-nl.md"), "owner"),
        "new-skill:foo bar",
    );
    add(&e, "tool.owner-config", "workflow", "config owner", "e").ok("add for config owner");
    e.run(&[
        "apply",
        "tool.owner-config",
        "--owner",
        "config:~/.zshenv",
        "--change-ref",
        "r2",
    ])
    .ok("apply config: owner");
    eq::<String, _>(
        "apply config owner recorded",
        fmval(p.join("applied/tool.owner-config.md"), "owner"),
        "config:~/.zshenv",
    );
    add(&e, "tool.owner-hint", "workflow", "hint owner", "e").ok("add for hint owner");
    e.run(&[
        "apply",
        "tool.owner-hint",
        "--owner",
        "hint:patch-reread",
        "--change-ref",
        "r3",
    ])
    .ok("apply hint: owner");
    eq::<String, _>(
        "apply hint owner recorded",
        fmval(p.join("applied/tool.owner-hint.md"), "owner"),
        "hint:patch-reread",
    );
    e.run(&[
        "apply",
        "nonexistent.key",
        "--owner",
        "nonsense",
        "--change-ref",
        "x",
    ])
    .die(
        "apply error text lists config: and hint:",
        "config:<path> | hint:<id>",
    );

    e.run(&[
        "apply",
        "test.wrong-cwd",
        "--owner",
        "skill:pytest",
        "--change-ref",
        "abc123",
    ])
    .ok("apply");
    let a = p.join("applied/test.wrong-cwd.md");
    check("apply moved to applied/", a.is_file() && !f.exists());
    eq::<String, _>("applied status", fmval(&a, "status"), "applied");
    eq::<String, _>("applied owner", fmval(&a, "owner"), "skill:pytest");
    eq::<String, _>("applied change-ref", fmval(&a, "change-ref"), "abc123");
    let at = fmval(&a, "applied-at");
    check(
        "applied applied-at",
        at.len() == 10
            && at.chars().enumerate().all(|(i, c)| {
                if i == 4 || i == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }
            }),
    );
    check(
        "applied key left INDEX",
        !rd(p.join("INDEX.md")).contains("test.wrong-cwd"),
    );

    e.run(&["bump", "test.wrong-cwd", "--evidence", "regression"])
        .die("bump on applied dies", "already applied")
        .has("bump on applied prints owner", "skill:pytest");
    eq::<String, _>(
        "applied file unchanged by failed bump",
        fmval(&a, "seen"),
        "3",
    );

    e.run(&["reject", "shell.no-sudo"])
        .die("reject requires reason", "usage");
    e.run(&[
        "reject",
        "shell.no-sudo",
        "--reason",
        "environment specific",
    ])
    .ok("reject");
    let r = p.join("rejected/shell.no-sudo.md");
    check(
        "reject moved to rejected/",
        r.is_file() && !p.join("candidates/shell.no-sudo.md").exists(),
    );
    eq::<String, _>("rejected status", fmval(&r, "status"), "rejected");
    eq::<String, _>(
        "rejected reason",
        fmval(&r, "reason"),
        "environment specific",
    );
    e.run(&["bump", "shell.no-sudo", "--evidence", "x"])
        .die("bump on rejected dies", "rejected");
}

const EVV: &str = "lacked packaging; project uses uv | a & b > c -> commit push revert";

#[test]
fn drafts_add_bump_reject() {
    let e = setup();
    let p = e.pdir();
    let drafts = e.store.join(".drafts");
    mkdirs(&drafts);
    let d1 = drafts.join("d1.md");
    wr(
        &d1,
        &format!(
            "key: git.draft-one\nkind: failure\nsummary: committed without tests: pushed to the wrong branch\nevidence: {EVV}\nproject: proj-d\nbehavior: Run tests before any commit; check the branch first.\nowner: skill:git\n"
        ),
    );
    e.run(&["add", "--from", d1.to_str().unwrap()])
        .ok("add --from draft");
    let d = p.join("candidates/git.draft-one.md");
    check("add --from: file created", d.is_file());
    eq::<String, _>(
        "add --from: summary keeps ': ' after first",
        fmval(&d, "description"),
        "committed without tests: pushed to the wrong branch",
    );
    eq::<String, _>("add --from: project", fmval(&d, "projects"), "[proj-d]");
    eq::<String, _>(
        "add --from: owner",
        fmval(&d, "suggested-owner"),
        "skill:git",
    );
    let ev = format!("- {} proj-d: {EVV}", today_local());
    eq::<usize, _>(
        "add --from: evidence stored verbatim, one line",
        rd(&d).lines().filter(|l| *l == ev).count(),
        1usize,
    );
    eq::<usize, _>(
        "add --from: evidence line is the only one",
        count_re(&d, "^- [0-9]{4}-"),
        1usize,
    );
    check("draft inside .drafts/ deleted after success", !d1.exists());

    let outside = e.root.join("outside-draft.md");
    wr(
        &outside,
        "key: git.draft-two\nkind: workflow\nsummary: outside draft\nevidence: e2\n",
    );
    e.run(&["add", "--from", outside.to_str().unwrap()])
        .ok("add --from draft outside .drafts");
    check("draft outside .drafts/ kept", outside.exists());

    // relative path resolves against $PWD; flags override the draft; ---block tolerated
    let rel = e.root.join("rel");
    mkdirs(&rel);
    wr(
        rel.join("d3.md"),
        "---\nkey: git.draft-three\nkind: workflow\nsummary: from draft\nevidence: from draft evidence\nevidence: second match ignored\n---\n",
    );
    let o = e
        .cmd(&[
            "add",
            "--profile",
            "work",
            "--from",
            "d3.md",
            "--summary",
            "from flag",
            "--kind",
            "correction",
        ])
        .cwd(&rel)
        .run();
    let t3 = p.join("candidates/git.draft-three.md");
    check(
        "add --from relative path (leading --- tolerated)",
        o.rc == 0 && t3.is_file(),
    );
    eq::<String, _>(
        "flag overrides draft summary",
        fmval(&t3, "description"),
        "from flag",
    );
    eq::<String, _>(
        "flag overrides draft kind",
        fmval(&t3, "kind"),
        "correction",
    );
    check(
        "draft first match per field wins",
        rd(&t3).contains("proj-a: from draft evidence\n"),
    );

    e.run(&["add", "--from", e.root.join("nope.md").to_str().unwrap()])
        .die("add --from missing file dies", "cannot read draft");
    e.run(&["bump", "--from", e.root.join("nope.md").to_str().unwrap()])
        .die("bump --from missing file dies", "cannot read draft");
    e.run(&["reject", "--from", e.root.join("nope.md").to_str().unwrap()])
        .die("reject --from missing file dies", "cannot read draft");

    let b1 = drafts.join("b1.md");
    wr(
        &b1,
        &format!("key: git.draft-three\nevidence: {EVV}\nproject: proj-e\n"),
    );
    e.run(&["bump", "--from", b1.to_str().unwrap()])
        .ok("bump --from draft (key from draft)");
    eq::<String, _>("bump --from: seen 2", fmval(&t3, "seen"), "2");
    eq::<String, _>(
        "bump --from: project added",
        fmval(&t3, "projects"),
        "[proj-a, proj-e]",
    );
    check(
        "bump --from: evidence verbatim",
        rd(&t3).contains(&format!("proj-e: {EVV}")),
    );
    check("bump --from: draft deleted", !b1.exists());

    let b2 = drafts.join("b2.md");
    wr(&b2, "key: git.draft-three\nevidence: draft ev\n");
    e.run(&[
        "bump",
        "git.draft-one",
        "--from",
        b2.to_str().unwrap(),
        "--evidence",
        "flag ev",
    ])
    .ok("bump positional key overrides draft");
    eq::<String, _>(
        "bump override: positional key bumped",
        fmval(&d, "seen"),
        "2",
    );
    check(
        "bump override: flag evidence used",
        rd(&d).contains("flag ev\n"),
    );

    let r1 = drafts.join("r1.md");
    wr(&r1, &format!("key: git.draft-one\nreason: wrong: {EVV}\n"));
    e.run(&["reject", "--from", r1.to_str().unwrap()])
        .ok("reject --from draft");
    let rj = p.join("rejected/git.draft-one.md");
    check("reject --from: moved", rj.is_file() && !d.exists());
    eq::<String, _>(
        "reject --from: reason stored",
        fmval(&rj, "reason"),
        format!("wrong: {EVV}"),
    );
    check("reject --from: draft deleted", !r1.exists());

    let r2 = drafts.join("r2.md");
    wr(&r2, "key: git.draft-three\nreason: from draft\n");
    e.run(&[
        "reject",
        "--from",
        r2.to_str().unwrap(),
        "--reason",
        "from flag",
    ])
    .ok("reject flag reason overrides draft");
    eq::<String, _>(
        "reject override: reason from flag",
        fmval(p.join("rejected/git.draft-three.md"), "reason"),
        "from flag",
    );
}

#[test]
fn dream_draft_restrictions() {
    let e = setup();
    let p = e.pdir();
    let drafts = e.store.join(".drafts");
    mkdirs(&drafts);
    let outside = e.root.join("dream-outside.md");
    wr(
        &outside,
        "key: git.dream-outside\nkind: workflow\nsummary: s\nevidence: e\n",
    );
    let o = e
        .cmd(&[
            "add",
            "--profile",
            "work",
            "--from",
            outside.to_str().unwrap(),
        ])
        .env("LEARN_DREAM", "1")
        .run();
    check(
        "dream: --from outside .drafts refused",
        o.rc != 0
            && o.out().contains("must point into")
            && !p.join("candidates/git.dream-outside.md").exists(),
    );
    let esc = drafts.join("escape.md");
    symlink(&outside, &esc).unwrap();
    let o = e
        .cmd(&["add", "--profile", "work", "--from", esc.to_str().unwrap()])
        .env("LEARN_DREAM", "1")
        .run();
    check(
        "dream: symlink in .drafts pointing outside refused",
        o.rc != 0 && o.out().contains("must point into"),
    );
    fs::remove_file(&esc).unwrap();
    fs::copy(&outside, drafts.join("dream-in.md")).unwrap();
    let o = e
        .cmd(&["add", "--profile", "work", "--from", ".drafts/dream-in.md"])
        .env("LEARN_DREAM", "1")
        .cwd(&e.store)
        .run();
    check(
        "dream: --from .drafts/ relative accepted",
        o.rc == 0 && p.join("candidates/git.dream-outside.md").is_file(),
    );
}

#[test]
fn symlink_cycle_draft_refused() {
    let e = setup();
    let cyc = e.root.join("cyc");
    mkdirs(&cyc);
    symlink(cyc.join("b"), cyc.join("a")).unwrap();
    symlink(cyc.join("a"), cyc.join("b")).unwrap();
    e.run(&[
        "add",
        "--key",
        "a.b",
        "--kind",
        "workflow",
        "--summary",
        "s",
        "--evidence",
        "e",
        "--from",
        cyc.join("a").to_str().unwrap(),
    ])
    .die("add --from symlink cycle errors", "cannot read draft file");
}

#[test]
fn dates() {
    let mut e = setup();
    let p = e.pdir();
    e.set("LEARN_TODAY", "2026-09-30");
    let dd1 = e.store.join(".drafts/dd1.md");
    wr(
        &dd1,
        "key: git.dated\nkind: failure\nsummary: dated event\nevidence: old event text\ndate: 2026-07-22\nproject: proj-a\n",
    );
    e.run(&["add", "--from", dd1.to_str().unwrap()])
        .ok("add --from with date:");
    let dd = p.join("candidates/git.dated.md");
    eq::<String, _>(
        "add date: first-seen is the event date",
        fmval(&dd, "first-seen"),
        "2026-07-22",
    );
    eq::<String, _>(
        "add date: last-seen is the event date",
        fmval(&dd, "last-seen"),
        "2026-07-22",
    );
    let ev: Vec<String> = rd(&dd)
        .lines()
        .filter(|l| l.starts_with("- "))
        .map(String::from)
        .collect();
    eq::<Vec<String>, _>(
        "add date: evidence prefixed once with event date",
        ev,
        vec!["- 2026-07-22 proj-a: old event text".to_string()],
    );
    e.run(&[
        "add",
        "--key",
        "git.dated-flag",
        "--kind",
        "failure",
        "--summary",
        "s",
        "--evidence",
        "e",
        "--date",
        "2026-08-01",
    ])
    .ok("add --date flag overrides draft date");
    let df = p.join("candidates/git.dated-flag.md");
    check(
        "add --date: first/last-seen",
        fmval(&df, "first-seen") == "2026-08-01" && fmval(&df, "last-seen") == "2026-08-01",
    );
    add(&e, "git.undated", "failure", "s", "e").ok("add without date uses today");
    eq::<String, _>(
        "add without date: LEARN_TODAY",
        fmval(p.join("candidates/git.undated.md"), "first-seen"),
        "2026-09-30",
    );
    e.run(&[
        "add",
        "--key",
        "git.baddate",
        "--kind",
        "failure",
        "--summary",
        "s",
        "--evidence",
        "e",
        "--date",
        "2026-02-30",
    ])
    .die("add rejects bad date", "invalid date");
    e.run(&[
        "add",
        "--key",
        "git.baddate",
        "--kind",
        "failure",
        "--summary",
        "s",
        "--evidence",
        "e",
        "--date",
        "yesterday",
    ])
    .die("add rejects non-date", "invalid date");

    e.run(&[
        "bump",
        "git.dated",
        "--evidence",
        "older",
        "--date",
        "2026-07-01",
    ])
    .ok("bump --date older");
    check(
        "bump older: first-seen min",
        fmval(&dd, "first-seen") == "2026-07-01" && fmval(&dd, "last-seen") == "2026-07-22",
    );
    e.run(&[
        "bump",
        "git.dated",
        "--evidence",
        "newer",
        "--date",
        "2026-08-15",
    ])
    .ok("bump --date newer");
    check(
        "bump newer: last-seen max",
        fmval(&dd, "first-seen") == "2026-07-01" && fmval(&dd, "last-seen") == "2026-08-15",
    );
    e.run(&[
        "bump",
        "git.dated",
        "--evidence",
        "middle",
        "--date",
        "2026-07-10",
    ])
    .ok("bump --date in between");
    check(
        "bump middle: range unchanged",
        fmval(&dd, "first-seen") == "2026-07-01" && fmval(&dd, "last-seen") == "2026-08-15",
    );
    check(
        "bump evidence lines use their own dates",
        rd(&dd).lines().any(|l| l == "- 2026-07-10 proj-a: middle"),
    );
    let dd2 = e.store.join(".drafts/dd2.md");
    wr(
        &dd2,
        "key: git.dated\nevidence: from draft\ndate: 2026-09-01\n",
    );
    e.run(&["bump", "--from", dd2.to_str().unwrap()])
        .ok("bump --from with date:");
    eq::<String, _>(
        "bump date: last-seen max",
        fmval(&dd, "last-seen"),
        "2026-09-01",
    );
    check(
        "bump date: evidence dated",
        rd(&dd)
            .lines()
            .any(|l| l == "- 2026-09-01 proj-a: from draft"),
    );
    e.run(&["bump", "git.dated", "--evidence", "no date"])
        .ok("bump without date uses today");
    eq::<String, _>(
        "bump without date: last-seen today",
        fmval(&dd, "last-seen"),
        "2026-09-30",
    );
    e.run(&[
        "bump",
        "git.dated",
        "--evidence",
        "x",
        "--date",
        "2026-13-01",
    ])
    .die("bump rejects bad date", "invalid date");
}
