//! markers, backup, install-opencode.
mod common;
use common::*;
use std::fs;

#[test]
fn markers_add_list_test() {
    let e = Env::new();
    e.init_work();
    let mj = e.store.join("markers.json");
    e.run(&["markers", "add", "--tier", "strong", "^тест-маркер"])
        .ok("markers add");
    check("marker persisted", rd(&mj).contains("тест-маркер"));
    e.run(&["markers", "add", "--tier", "strong", "^тест-маркер"])
        .has("markers add is idempotent", "already present");
    e.run(&["markers", "add", "--tier", "bogus", "x"])
        .die("markers add rejects bad tier", "usage");
    let o = e.run(&["markers", "add", "--tier", "weak", "("]);
    check(
        "markers add rejects bad regex without persisting",
        o.rc != 0 && !rd(&mj).contains("\"(\""),
    );
    let stray = fs::read_dir(&e.store).unwrap().flatten().any(|d| {
        d.file_name()
            .to_string_lossy()
            .starts_with("markers.json.tmp.")
    });
    check("no stray markers tmp file", !stray);
    e.run(&["markers", "list"])
        .ok("markers list")
        .has("markers list groups tiers", "correction/strong:")
        .has("markers list shows personal", "тест-маркер");
    let o = e.run(&["markers", "test", "Нет, я про другое"]);
    check(
        "markers test -> strong",
        o.rc == 0 && o.stdout.starts_with("strong"),
    );
    let o = e.run(&["markers", "test", "просто нейтральная фраза без маркеров"]);
    check(
        "markers test -> no match",
        o.rc == 0 && o.out().contains("no match"),
    );
    let o = e.run(&["markers", "test", "ок"]);
    check(
        "markers test -> confirmation",
        o.stdout.starts_with("confirmation\t"),
    );
}

fn backup_fixture() -> Env {
    let e = Env::new();
    e.init_work();
    e.run(&[
        "add",
        "--key",
        "test.a",
        "--kind",
        "correction",
        "--summary",
        "s",
        "--evidence",
        "e",
    ])
    .ok("add");
    e.run(&[
        "add",
        "--key",
        "test.b",
        "--kind",
        "correction",
        "--summary",
        "s",
        "--evidence",
        "e",
    ])
    .ok("add");
    e.run(&["bump", "test.a", "--evidence", "again"]).ok("bump");
    e.run(&["apply", "test.b", "--owner", "skill:x", "--change-ref", "r"])
        .ok("apply");
    e
}

#[test]
fn backup_commits_and_counts() {
    let e = backup_fixture();
    e.run(&["backup"])
        .ok("backup")
        .has("backup message has profile", "learn(work):");
    let last = e.git_out(&e.store, &["log", "-1", "--format=%s"]);
    check(
        "backup commit message format",
        last.starts_with("learn(work): +")
            && last.contains(" candidates, ~")
            && last.contains(" bumped, →")
            && last.contains(" applied"),
    );
    check(
        "store clean after backup",
        e.git_out(&e.store, &["status", "--porcelain"]).is_empty(),
    );
    e.run(&["backup"])
        .ok("backup with nothing to commit")
        .has("backup reports nothing", "nothing to back up");
    e.run(&[
        "add",
        "--key",
        "build.after-backup",
        "--kind",
        "workflow",
        "--summary",
        "post backup",
        "--evidence",
        "e",
    ])
    .ok("add");
    e.run(&["backup"]).ok("backup second commit");
    let last = e.git_out(&e.store, &["log", "-1", "--format=%s"]);
    check(
        "backup counts new candidate",
        last.starts_with("learn(work): +1 candidates, ~0 bumped, →0 applied"),
    );
    wr(e.pdir().join("candidates/.learn.leftover"), "");
    e.run(&["backup"]).ok("backup with stray temp file");
    check(
        "temp file excluded from the store",
        e.git_out(&e.store, &["ls-files", "*.learn.*"]).is_empty(),
    );
    check(
        "temp file excluded in info/exclude",
        rd(e.store.join(".git/info/exclude"))
            .lines()
            .any(|l| l == ".learn.*"),
    );
    e.run(&["backup", "--push"])
        .die("backup --push without remote dies", "no remote");
}

#[test]
fn install_opencode() {
    let e = Env::new();
    e.init_work();
    e.run(&["install-opencode"])
        .ok("install-opencode")
        .has("prints deprecation note", "opencode plugin add");
    let link = e.home.join(".config/opencode/plugins/self-improving.js");
    check(
        "plugin symlink created",
        fs::symlink_metadata(&link)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
    );
    e.run(&["install-opencode"])
        .ok("install-opencode idempotent")
        .has("reports already installed", "already installed");
}
