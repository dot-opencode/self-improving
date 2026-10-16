//! doctor / status / grep / --profile / profile resolution.
mod common;
use common::*;
use std::fs;

fn setup() -> Env {
    let e = Env::new();
    e.init_work();
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
    ])
    .ok("add");
    e.run(&[
        "apply",
        "test.wrong-cwd",
        "--owner",
        "skill:pytest",
        "--change-ref",
        "abc123",
    ])
    .ok("apply");
    e.run(&[
        "add",
        "--key",
        "tool.owner-nl",
        "--kind",
        "workflow",
        "--summary",
        "s",
        "--evidence",
        "e",
    ])
    .ok("add");
    e.run(&[
        "apply",
        "tool.owner-nl",
        "--owner",
        "new-skill:foo",
        "--change-ref",
        "r1",
    ])
    .ok("apply");
    e.run(&["index"]).ok("index");
    e
}

#[test]
fn doctor_checks() {
    let e = setup();
    let p = e.pdir();
    let o = e.run(&["doctor"]);
    check("doctor exits 0", o.rc == 0 && o.out().contains("OK"));
    let a = p.join("applied/test.wrong-cwd.md");
    let orig = rd(&a);
    // missing owner
    let blanked: String = orig
        .lines()
        .map(|l| if l.starts_with("owner: ") { "" } else { l })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    wr(&a, &blanked);
    e.run(&["doctor"]).die(
        "doctor flags applied missing owner",
        "APPLIED MISSING owner",
    );
    wr(&a, &orig);
    e.run(&["doctor"]).ok("doctor clean after owner restored");
    e.run(&["doctor"]).ok("doctor accepts new-skill: owner");
    e.run(&["doctor"])
        .ok("doctor accepts config: and hint: owners");

    wr(e.store.join("hints.json"), "{\"version\":1,\"hints\":[{\"id\":\"x\",\"tools\":[\"grep\"],\"match\":\"(\",\"text\":\"t\"}]}\n");
    e.run(&["doctor"])
        .die("doctor flags bad personal hints.json", "BAD HINTS");
    wr(e.store.join("hints.json"), "{\"version\":1,\"hints\":[{\"id\":\"x\",\"tools\":[\"grep\"],\"match\":\"boom\",\"text\":\"t\"}]}\n");
    e.run(&["doctor"])
        .ok("doctor accepts valid personal hints.json");
    fs::remove_file(e.store.join("hints.json")).unwrap();

    let nl = p.join("applied/tool.owner-nl.md");
    let nl_orig = rd(&nl);
    let bogus: String = nl_orig
        .lines()
        .map(|l| {
            if l.starts_with("owner: ") {
                "owner: bogus:x"
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    wr(&nl, &bogus);
    e.run(&["doctor"])
        .die("doctor flags invalid owner", "BAD OWNER");
    wr(&nl, &nl_orig);
    let idx = p.join("INDEX.md");
    wr(&idx, &(rd(&idx) + "drift\n"));
    e.run(&["doctor"])
        .die("doctor flags INDEX drift", "INDEX DRIFT");
    e.run(&["index"]).ok("index");
    e.run(&["doctor"]).ok("doctor clean again");
}

#[test]
fn status_and_grep() {
    let e = setup();
    e.run(&["status"])
        .ok("status")
        .has("status shows profile", "work")
        .has("status shows git state", "git:");
    e.run(&["grep", "ModuleNotFoundError"])
        .ok("grep finds text")
        .has("grep hit path", "applied/test.wrong-cwd.md");
    e.run(&["grep", "definitely-not-present-xyz"])
        .ok("grep no match still exits 0")
        .has("grep no match message", "no matches");
}

#[test]
fn leading_trailing_profile() {
    let e = setup();
    e.run(&["--profile", "work", "status"])
        .ok("leading --profile <p> status")
        .has("leading --profile status output", "work");
    e.run(&["--profile=work", "status"])
        .ok("leading --profile=<p> status");
    e.run(&["status", "--profile", "work"])
        .ok("trailing status --profile <p>");
    e.run(&["--profile"]).die(
        "leading --profile without value",
        "--profile requires a value",
    );
    e.run(&["--profile", "nope", "status"])
        .die("leading --profile unknown profile", "no such profile");
}

#[test]
fn commands_die_without_learnings() {
    let e = setup();
    let other = e.root.join("other");
    mkdirs(&other);
    let o = e.cmd(&["status"]).cwd(&other).run();
    check("commands die without .learnings", o.rc != 0);
}

#[test]
fn grep_options() {
    let e = setup();
    e.run(&["grep", "modulenotfound", "-l"])
        .ok("grep -l")
        .has("lists file", "applied/test.wrong-cwd.md");
    e.run(&["grep", "->", "-F"])
        .ok("grep -F literal")
        .has("literal hit", "ModuleNotFoundError");
    e.run(&["grep", "pytest", "-c"]).ok("grep -c");
    e.run(&["grep", "pytest", "-A", "2"]).die(
        "unsupported grep option is a clear error",
        "unsupported grep option",
    );
}
