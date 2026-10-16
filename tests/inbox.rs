//! inbox / ack / prune, --limit ties, spaced JSON, ats arrival key.
mod common;
use common::*;

fn dated_lines(o: &Out) -> Vec<String> {
    lines_with(&o.out(), "2026-")
}

fn first_fields(o: &Out) -> String {
    dated_lines(o)
        .iter()
        .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn seed(e: &Env) {
    let p = e.pdir();
    wr(
        p.join("inbox/2026-09-29.jsonl"),
        "{\"ts\":\"2026-09-29T10:00:00Z\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"exit 2: pytest: ModuleNotFoundError\",\"project\":\"proj-a\",\"session\":\"ses_1\"}\n\
{\"ts\":\"2026-09-29T10:05:00Z\",\"kind\":\"correction\",\"tier\":\"strong\",\"marker\":\"^нет[,.]\",\"snippet\":\"Нет, я про другое\",\"project\":\"proj-a\",\"session\":\"ses_1\",\"prev_tool\":\"edit\"}\n",
    );
    wr(
        p.join("inbox/2026-09-30.jsonl"),
        "{\"ts\":\"2026-09-30T09:00:00Z\",\"kind\":\"correction\",\"tier\":\"weak\",\"marker\":\"^почему\",\"snippet\":\"Почему ci упал?\",\"project\":\"proj-b\",\"session\":\"ses_2\",\"prev_tool\":\"bash\"}\n\
{\"ts\":\"2026-09-30T09:30:00Z\",\"kind\":\"confirmation\",\"tier\":\"\",\"marker\":\"^ок\",\"snippet\":\"ок\",\"project\":\"proj-b\",\"session\":\"ses_2\"}\n",
    );
}

#[test]
fn inbox_filters_and_ack() {
    let e = Env::new();
    e.init_work();
    seed(&e);
    let p = e.pdir();

    let o = e.run(&["inbox"]);
    eq::<usize, _>(
        "inbox lists all 4 events before ack",
        o.count("2026-"),
        4usize,
    );
    o.has("inbox compact shows failure text", "bash: exit 2: pytest")
        .has("inbox compact shows snippet", "Нет, я про другое");
    eq::<usize, _>(
        "inbox --kind failure",
        e.run(&["inbox", "--kind", "failure"]).count("2026-"),
        1usize,
    );
    eq::<usize, _>(
        "inbox --tier weak",
        e.run(&["inbox", "--tier", "weak"]).count("2026-"),
        1usize,
    );
    eq::<usize, _>(
        "inbox --since",
        e.run(&["inbox", "--since", "2026-09-30T00:00:00Z"])
            .count("2026-"),
        2usize,
    );
    let o = e.run(&["inbox", "--json"]);
    eq::<usize, _>("inbox --json raw lines", o.count("{"), 4usize);
    o.lacks("inbox --json has no ack trailer", "ack: learn ack");
    e.run(&["inbox"]).has(
        "inbox trailer has max ts",
        "ack: learn ack --until 2026-09-30T09:30:00Z",
    );
    e.run(&["inbox", "--kind", "failure"]).has(
        "inbox trailer follows filtered max ts",
        "ack: learn ack --until 2026-09-29T10:00:00Z",
    );
    let o = e.run(&["inbox", "--limit", "2"]);
    eq::<usize, _>("inbox --limit 2", o.count("2026-"), 2usize);
    let last = dated_lines(&o)
        .last()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    o.has(
        "inbox --limit trailer is max of the shown batch",
        &format!("ack: learn ack --until {last}"),
    );
    eq::<usize, _>(
        "inbox --json --limit 1",
        e.run(&["inbox", "--json", "--limit", "1"]).count("{"),
        1usize,
    );
    e.run(&["inbox", "--limit", "0"])
        .die("inbox --limit rejects 0", "positive integer");

    e.run(&["ack", "--until", "2026-09-29T23:59:59Z"])
        .ok("ack --until");
    eq::<String, _>(
        ".ack file written",
        rd(p.join("inbox/.ack")).trim_end().to_string(),
        "2026-09-29T23:59:59Z",
    );
    let o = e.run(&["inbox"]);
    eq::<usize, _>("inbox honors .ack", o.count("2026-"), 2usize);
    o.lacks("acked events hidden", "Нет, я про другое");
    eq::<usize, _>(
        "inbox --all ignores .ack",
        e.run(&["inbox", "--all"]).count("2026-"),
        4usize,
    );
    e.run(&["status"])
        .ok("status counts unacked")
        .has("status unacked count", "2 unacked");
    e.run(&["ack"]).ok("ack now");
    let o = e.run(&["inbox"]);
    o.has("inbox empty after ack", "(no events)")
        .lacks("no trailer when no events", "ack: learn ack");
    e.run(&["ack", "--until", "yesterday"])
        .die("ack rejects bad timestamp", "invalid timestamp");
    e.run(&["ack", "--until", "2026-09-30T12:00:00"]).die(
        "ack rejects timestamp without timezone",
        "invalid timestamp",
    );
    e.run(&["ack", "--until", "2026-09-30T12:00:00+03:00"])
        .ok("ack accepts offset timestamp");
    e.run(&["ack", "--until", "2026-09-30"])
        .ok("ack date-only normalizes to UTC");
    eq::<String, _>(
        "ack date-only stored as Z",
        rd(p.join("inbox/.ack")).trim_end().to_string(),
        "2026-09-30T00:00:00Z",
    );
}

#[test]
fn prune_candidates_and_inbox_files() {
    let mut e = Env::new();
    e.init_work();
    seed(&e);
    let p = e.pdir();
    let add = |day: &str, args: &[&str]| {
        e.cmd(args).env("LEARN_TODAY", day).run();
    };
    add(
        "2026-01-01",
        &[
            "add",
            "--key",
            "env.stale-thing",
            "--kind",
            "workflow",
            "--summary",
            "old one-off",
            "--evidence",
            "old",
        ],
    );
    add(
        "2026-09-30",
        &[
            "add",
            "--key",
            "env.fresh-thing",
            "--kind",
            "workflow",
            "--summary",
            "fresh one-off",
            "--evidence",
            "new",
        ],
    );
    add(
        "2026-01-01",
        &[
            "add",
            "--key",
            "env.old-repeated",
            "--kind",
            "workflow",
            "--summary",
            "old but repeated",
            "--evidence",
            "old",
        ],
    );
    add(
        "2026-01-02",
        &["bump", "env.old-repeated", "--evidence", "again"],
    );
    e.set("LEARN_TODAY", "2026-09-30");
    e.run(&["prune", "--days", "90"]).ok("prune --days 90");
    let rj = p.join("rejected/env.stale-thing.md");
    check("stale seen=1 moved to rejected", rj.is_file());
    eq::<String, _>("stale reason: stale", fmval(&rj, "reason"), "stale");
    eq::<String, _>("stale status: rejected", fmval(&rj, "status"), "rejected");
    check(
        "fresh candidate kept",
        p.join("candidates/env.fresh-thing.md").is_file(),
    );
    check(
        "seen>1 candidate kept",
        p.join("candidates/env.old-repeated.md").is_file(),
    );
    check(
        "pruned key left INDEX",
        !rd(p.join("INDEX.md")).contains("env.stale-thing"),
    );

    wr(p.join("inbox/2026-05-01.jsonl"), "{\"ts\":\"2026-05-01T10:00:00Z\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"old\",\"project\":\"p\"}\n");
    wr(p.join("inbox/2026-08-01.jsonl"), "{\"ts\":\"2026-08-01T10:00:00Z\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"mid\",\"project\":\"p\"}\n");
    e.run(&["ack", "--until", "2026-09-30T00:00:00Z"]);
    e.run(&["prune", "--days", "90"]).has(
        "prune reports removed inbox count",
        "removed 1 acked inbox file(s)",
    );
    check(
        "old acked inbox file removed",
        !p.join("inbox/2026-05-01.jsonl").exists(),
    );
    check(
        "recent acked inbox file kept",
        p.join("inbox/2026-08-01.jsonl").is_file(),
    );
    check(
        "newest inbox files kept",
        p.join("inbox/2026-09-29.jsonl").is_file() && p.join("inbox/2026-09-30.jsonl").is_file(),
    );
    e.run(&["prune", "--days", "30"]).has(
        "prune --days 30 removes more",
        "removed 1 acked inbox file(s)",
    );
    check(
        "mid-age inbox file removed at --days 30",
        !p.join("inbox/2026-08-01.jsonl").exists(),
    );
    wr(p.join("inbox/2026-05-02-backfill.jsonl"), "{\"ts\":\"2026-01-01T10:00:00Z\",\"ats\":\"2026-05-02T10:00:00Z\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"bf-old\",\"project\":\"p\"}\n");
    wr(p.join("inbox/2026-09-29-backfill.jsonl"), "{\"ts\":\"2026-01-01T10:00:00Z\",\"ats\":\"2026-09-29T10:00:00Z\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"bf-new\",\"project\":\"p\"}\n");
    e.run(&["prune", "--days", "30"]).has(
        "prune counts an old backfill file",
        "removed 1 acked inbox file(s)",
    );
    check(
        "old acked backfill file removed",
        !p.join("inbox/2026-05-02-backfill.jsonl").exists(),
    );
    check(
        "recent backfill file kept",
        p.join("inbox/2026-09-29-backfill.jsonl").is_file(),
    );
    std::fs::remove_file(p.join("inbox/2026-09-29-backfill.jsonl")).unwrap();
    e.unset("LEARN_TODAY");
    e.run(&["doctor"]).ok("doctor clean after prune");
}

fn tev(ts: &str, head: &str) -> String {
    format!("{{\"ts\":\"2026-09-30T{ts}\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"{head}\",\"project\":\"proj-a\"}}\n")
}

#[test]
fn limit_ties_and_line_order() {
    let e = Env::new();
    let ties = e.root.join("ties");
    bare_store(&e, &ties, "work");
    wr(
        ties.join("work/inbox/2026-09-30.jsonl"),
        &[
            tev("10:00:01Z", "a1"),
            tev("10:00:02Z", "b1"),
            tev("10:00:02Z", "b2"),
            tev("10:00:03Z", "c1"),
            tev("10:00:00Z", "late"),
        ]
        .concat(),
    );
    let tl = |args: &[&str]| e.cmd(args).env("AGENT_LEARNINGS_HOME", &ties).run();
    let o = tl(&["inbox", "--profile", "work", "--limit", "2"]);
    eq::<usize, _>("inbox --limit 2: two events", o.count("2026-"), 2usize);
    eq::<String, _>(
        "inbox --limit 2: sorted by ts, late-appended line first",
        first_fields(&o),
        "2026-09-30T10:00:00Z 2026-09-30T10:00:01Z",
    );
    o.has(
        "inbox --limit 2: trailer is 10:00:01",
        "ack: learn ack --until 2026-09-30T10:00:01Z",
    );
    let o = tl(&["inbox", "--profile", "work", "--limit", "3"]);
    eq::<usize, _>(
        "inbox --limit 3 extends the tie (4 events)",
        o.count("2026-"),
        4usize,
    );
    o.has(
        "inbox --limit 3: trailer is 10:00:02",
        "ack: learn ack --until 2026-09-30T10:00:02Z",
    )
    .lacks("inbox --limit 3: next ts not shown", "10:00:03Z  ");
    let o = tl(&["inbox", "--profile", "work", "--json", "--limit", "3"]);
    eq::<usize, _>(
        "inbox --json --limit 3 extends the tie (4 lines)",
        o.count("{"),
        4usize,
    );
    check(
        "inbox --json --limit 3: first line is 10:00:00",
        o.out()
            .lines()
            .next()
            .unwrap()
            .contains("\"ts\":\"2026-09-30T10:00:00Z\""),
    );
    eq::<usize, _>(
        "inbox --json without limit keeps all 5",
        tl(&["inbox", "--profile", "work", "--json"]).count("{"),
        5usize,
    );
    tl(&[
        "ack",
        "--profile",
        "work",
        "--until",
        "2026-09-30T10:00:02Z",
    ]);
    let o = tl(&["inbox", "--profile", "work"]);
    check(
        "after ack --until 10:00:02 only 10:00:03 remains",
        o.count("2026-") == 1 && o.out().contains("2026-09-30T10:00:03Z"),
    );
}

#[test]
fn spaced_json_lines() {
    let e = Env::new();
    let s = e.root.join("sstore");
    bare_store(&e, &s, "work");
    wr(
        s.join("work/inbox/2026-07-22.jsonl"),
        "{\"ts\": \"2026-07-22T10:00:00Z\", \"kind\": \"failure\", \"tool\": \"bash\", \"head\": \"exit 1: boom\", \"project\": \"proj-a\"}\n",
    );
    let o = e
        .cmd(&["inbox", "--profile", "work"])
        .env("AGENT_LEARNINGS_HOME", &s)
        .run();
    o.has(
        "inbox parses spaced JSON",
        "2026-07-22T10:00:00Z  failure  proj-a  bash: exit 1: boom",
    )
    .has(
        "inbox trailer from spaced JSON",
        "ack: learn ack --until 2026-07-22T10:00:00Z",
    );
    e.cmd(&["dream", "--report"])
        .env("AGENT_LEARNINGS_HOME", &s)
        .run()
        .has(
            "dream --report oldest/newest from spaced JSON",
            "oldest 2026-07-22T10:00:00Z, newest 2026-07-22T10:00:00Z",
        );
}

fn aev(ts: &str, head: &str) -> String {
    format!("{{\"ts\":\"{ts}\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"{head}\",\"project\":\"proj-a\"}}\n")
}

fn aev2(ts: &str, ats: &str, head: &str) -> String {
    format!("{{\"ts\":\"{ts}\",\"ats\":\"{ats}\",\"kind\":\"failure\",\"tool\":\"bash\",\"head\":\"{head}\",\"project\":\"proj-a\"}}\n")
}

#[test]
fn ats_arrival_key() {
    let e = Env::new();
    let a = e.root.join("astore");
    bare_store(&e, &a, "work");
    wr(
        a.join("work/inbox/2026-09-30.jsonl"),
        &[
            aev("2026-09-30T10:00:00Z", "plain1"),
            aev2("2026-01-05T08:00:00Z", "2026-09-30T12:00:00Z", "backfilled"),
            aev("2026-09-30T11:00:00Z", "plain2"),
        ]
        .concat(),
    );
    let al = |args: &[&str]| e.cmd(args).env("AGENT_LEARNINGS_HOME", &a).run();
    al(&[
        "ack",
        "--profile",
        "work",
        "--until",
        "2026-09-30T10:30:00Z",
    ]);
    let o = al(&["inbox", "--profile", "work"]);
    eq::<usize, _>(
        "ats: old-ts event with new ats shown after ack",
        o.count("2026-"),
        2usize,
    );
    eq::<String, _>(
        "ats: ordered by ats, ts column still shown",
        first_fields(&o),
        "2026-09-30T11:00:00Z 2026-01-05T08:00:00Z",
    );
    o.has(
        "ats: trailer prints max ats",
        "ack: learn ack --until 2026-09-30T12:00:00Z",
    );
    let o = al(&["inbox", "--profile", "work", "--json"]);
    check(
        "ats: --json prints raw lines",
        o.count("{") == 2 && o.out().contains("\"ats\":\"2026-09-30T12:00:00Z\""),
    );
    eq::<usize, _>(
        "ats: --since filters on ts",
        al(&[
            "inbox",
            "--profile",
            "work",
            "--since",
            "2026-06-01T00:00:00Z",
        ])
        .count("2026-"),
        1usize,
    );
    al(&["dream", "--report"]).has(
        "ats: dream --report counts by key, oldest/newest by ts",
        "2 event(s), oldest 2026-01-05T08:00:00Z, newest 2026-09-30T11:00:00Z",
    );
    al(&[
        "ack",
        "--profile",
        "work",
        "--until",
        "2026-09-30T12:00:00Z",
    ]);
    al(&["inbox", "--profile", "work"])
        .has("ats: acking up to max ats hides the event", "(no events)");
    wr(
        a.join("work/inbox/2026-10-01-backfill.jsonl"),
        &[
            aev2("2026-01-01T00:00:00Z", "2026-10-01T00:00:01Z", "t1"),
            aev2("2026-01-02T00:00:00Z", "2026-10-01T00:00:02Z", "t2"),
            aev2("2026-01-03T00:00:00Z", "2026-10-01T00:00:02Z", "t3"),
            aev("2026-10-01T00:00:03Z", "t4"),
        ]
        .concat(),
    );
    let o = al(&["inbox", "--profile", "work", "--limit", "2"]);
    eq::<usize, _>(
        "ats: --limit 2 extends tie on ats (3 events)",
        o.count("2026-"),
        3usize,
    );
    o.has(
        "ats: --limit trailer is max ats of batch",
        "ack: learn ack --until 2026-10-01T00:00:02Z",
    )
    .lacks("ats: --limit excludes next key", "t4");
}
