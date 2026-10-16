//! `learn`: Rust port of skills/self-improving/bin/learn (bash, v0.1.0).
//!
//! Locating assets: the default markers/hints JSON, the OpenCode plugin and the
//! dream config live in the skill directory (skills/self-improving). It is
//! resolved from `LEARN_SKILL_DIR` first (the release shim exports it), then
//! relative to the executable's canonical path (see `store::find_skill_dir`).

mod cmds;
mod dream;
mod git;
mod grep;
mod inbox;
mod markers;
mod store;
mod util;

use store::Ctx;
use util::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = r#"usage: learn <command> [args]        (learn --version prints the version)

commands:
  init [profile]                            set up the store and .learnings/ for this project (user-only)
  inbox [--since <ts>] [--kind k] [--tier t] [--all] [--json] [--limit N]
                                            show plugin events (unacked by default)
  ack [--until <ts>]                        mark the inbox as processed
  add --key <area.symptom> --kind <k> --summary "<=120" --evidence "..." [--project p] [--behavior "..."] [--owner o] [--date YYYY-MM-DD]
  add --from <file> [flags]                 create a candidate (seen: 1); the draft holds flat
                                            `field: value` lines (key kind summary evidence project
                                            behavior owner date); flags override it; a draft inside
                                            <store>/.drafts/ is deleted after success.
                                            --date is the event date (default today): it prefixes
                                            the evidence line and sets first-seen/last-seen
  bump <key> --evidence "..." [--project p] [--date YYYY-MM-DD]
                                            another sighting of a candidate (first-seen = min,
                                            last-seen = max of the dates)
  bump [<key>] --from <file> [flags]        same, from a draft (key evidence project date)
  propose <key> [--owner <o>]               status: proposed
  apply <key> --owner <o> --change-ref <r>  move to applied/ (only after user approval)
                                            owner: script:<path> | skill:<name> | new-skill:<name>
                                            | agents:<repo> | config:<path> | hint:<id>
  reject <key> --reason "..."               move to rejected/
  reject [<key>] --from <file> [--reason r] same, from a draft (key reason)
  index                                     rebuild INDEX.md from candidates/
  status                                    profile, counts, unacked inbox, git state
  doctor                                    check symlink, frontmatter, index, markers
  grep <pattern> [grep opts]                search candidates/applied/rejected
  prune [--days 90]                         seen=1 and idle candidates -> rejected (reason: stale);
                                            also deletes acked inbox/*.jsonl older than --days
  markers list | add --tier <t> "<re>" | test "<text>"
  dream --report [--stale-days N] [--profile <p>]
                                            read-only whole-store report: unacked inbox, similar
                                            candidates, ready for proposal, stale, unusable
  dream [--profile <p>] [--model <provider/model>] [--if-changed] [--print-config]
                                            consolidate the store with a sandboxed OpenCode run
                                            (AGENT_LEARNINGS_DREAM_MODEL sets the default model)
  backup [--push]                           commit (and push) the store
  install-opencode                          install the global OpenCode plugin

profile-bound commands (inbox ack add bump propose apply reject index status doctor
grep prune) take `--profile <p>` as their first argument to address a store profile
without a ./.learnings symlink, e.g. `learn inbox --profile work --all`.
"#;

fn main() {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut ctx = Ctx::new();
    run(&mut ctx, &argv);
    exit_with(0);
}

fn run(ctx: &mut Ctx, argv: &[String]) {
    let mut rest: &[String] = argv;
    let mut cmd = String::new();
    if let Some(first) = rest.first() {
        cmd = first.clone();
        rest = &rest[1..];
    }
    // `--profile <p>` as the first argument addresses a store profile directly
    // (the dream agent runs in the store root, where there is no ./.learnings).
    if let Some(v) = cmd.strip_prefix("--profile=") {
        if v.is_empty() {
            die("--profile requires a value");
        }
        ctx.profile_override = v.to_string();
        cmd = rest.first().cloned().unwrap_or_default();
        if !rest.is_empty() {
            rest = &rest[1..];
        }
    } else if cmd == "--profile" {
        if rest.len() < 2 || rest[0].is_empty() {
            die("--profile requires a value");
        }
        ctx.profile_override = rest[0].clone();
        rest = &rest[1..];
        cmd = rest.first().cloned().unwrap_or_default();
        if !rest.is_empty() {
            rest = &rest[1..];
        }
    }
    if matches!(
        cmd.as_str(),
        "inbox"
            | "ack"
            | "add"
            | "bump"
            | "propose"
            | "apply"
            | "reject"
            | "index"
            | "status"
            | "doctor"
            | "grep"
            | "prune"
    ) && rest.first().map(|s| s.as_str()) == Some("--profile")
    {
        if rest.len() < 2 || rest[1].is_empty() {
            die("--profile requires a value");
        }
        ctx.profile_override = rest[1].clone();
        rest = &rest[2..];
    }
    match cmd.as_str() {
        "init" => cmds::cmd_init(ctx, rest),
        "inbox" => cmds::cmd_inbox(ctx, rest),
        "ack" => cmds::cmd_ack(ctx, rest),
        "add" => cmds::cmd_add(ctx, rest),
        "bump" => cmds::cmd_bump(ctx, rest),
        "propose" => cmds::cmd_propose(ctx, rest),
        "apply" => cmds::cmd_apply(ctx, rest),
        "reject" => cmds::cmd_reject(ctx, rest),
        "index" => cmds::cmd_index(ctx, rest),
        "status" => cmds::cmd_status(ctx, rest),
        "doctor" => cmds::cmd_doctor(ctx, rest),
        "grep" => cmds::cmd_grep(ctx, rest),
        "prune" => cmds::cmd_prune(ctx, rest),
        "markers" => cmds::cmd_markers(ctx, rest),
        "dream" => dream::cmd_dream(ctx, rest),
        "backup" => cmds::cmd_backup(ctx, rest),
        "install-opencode" => cmds::cmd_install_opencode(ctx, rest),
        "-v" | "--version" => {
            if !rest.is_empty() {
                die("usage: learn --version");
            }
            println!("learn {VERSION}");
        }
        "-h" | "--help" | "help" => {
            if !rest.is_empty() {
                die("usage: learn help");
            }
            print!("{USAGE}");
        }
        "" => {
            eprint!("{USAGE}");
            exit_with(1);
        }
        _ => die(&format!("unknown command: {cmd} (see 'learn help')")),
    }
}
