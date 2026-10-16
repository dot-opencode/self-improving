//! Command implementations (everything except `dream`).

use crate::dream;
use crate::git;
use crate::inbox::{self, Filter};
use crate::markers;
use crate::store::*;
use crate::util::*;
use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

/// Value following a flag at `args[*i]`; advances `i` by two.
pub fn val(args: &[String], i: &mut usize, flag: &str) -> String {
    if *i + 1 >= args.len() {
        die(&format!("{flag} requires a value"));
    }
    let v = args[*i + 1].clone();
    *i += 2;
    v
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

fn ensure_global_gitignore() {
    let (_, mut f) = git::out(
        None,
        &["config", "--global", "--path", "--get", "core.excludesfile"],
        true,
    );
    if f.is_empty() {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| format!("{}/.config", home_dir()));
        f = format!("{base}/git/ignore");
    } else if !f.starts_with('/') {
        let (_, origin) = git::out(
            None,
            &[
                "config",
                "--global",
                "--show-origin",
                "--get",
                "core.excludesfile",
            ],
            true,
        );
        let source = origin.split('\t').next().unwrap_or("");
        let source = source.strip_prefix("file:").unwrap_or(source);
        if source.is_empty() {
            die("cannot resolve global core.excludesfile");
        }
        f = format!("{}/{}", dirname(source), f);
    }
    if let Some(rest) = f.strip_prefix('~') {
        f = format!("{}{}", home_dir(), rest);
    }
    let existing = if is_file(&f) { read_text(&f) } else { None };
    let has = |text: &str, pat: &str| text.split('\n').any(|l| l == pat);
    if let Some(t) = &existing {
        if has(t, ".learnings") {
            return;
        }
    }
    mkdir_p(&dirname(&f));
    if let Some(t) = &existing {
        if has(t, ".learnings/") {
            let mut out = String::new();
            for l in lines_of(t) {
                out.push_str(if l == ".learnings/" { ".learnings" } else { l });
                out.push('\n');
            }
            write_atomic(&dirname(&f), &f, out.as_bytes());
            git::must(git::code(
                None,
                &["config", "--global", "core.excludesfile", &f],
                false,
            ));
            println!(
                "fixed global gitignore ({f}): '.learnings/' -> '.learnings' (it is a symlink)"
            );
            return;
        }
    }
    let mut bytes = fs::read(&f).unwrap_or_default();
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(b".learnings\n");
    if fs::write(&f, bytes).is_err() {
        die(&format!("cannot write {f}"));
    }
    git::must(git::code(
        None,
        &["config", "--global", "core.excludesfile", &f],
        false,
    ));
    println!("added '.learnings' to global gitignore ({f})");
}

pub fn cmd_init(ctx: &mut Ctx, args: &[String]) {
    let mut profile = String::new();
    for a in args {
        if a.starts_with('-') {
            die(&format!("unknown option: {a}"));
        }
        if !profile.is_empty() {
            die("usage: learn init <profile>");
        }
        profile = a.clone();
    }
    if profile.is_empty() {
        let mut err = String::from("existing profiles:\n");
        let mut found = false;
        if is_dir(&ctx.learn_home) {
            let mut names: Vec<String> = Vec::new();
            if let Ok(rd) = fs::read_dir(&ctx.learn_home) {
                for e in rd.flatten() {
                    let n = e.file_name().to_string_lossy().into_owned();
                    if !n.starts_with('.') && is_dir(&format!("{}/{}", ctx.learn_home, n)) {
                        names.push(n);
                    }
                }
            }
            names.sort();
            for n in names {
                err.push_str(&format!("  {n}\n"));
                found = true;
            }
        }
        if !found {
            err.push_str("  (none)\n");
        }
        let target = fs::read_link(".opencode")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if target.contains("/opencode/") {
            let after = &target[target.rfind("/opencode/").unwrap() + "/opencode/".len()..];
            let suggestion = after.split('/').next().unwrap_or("");
            if !suggestion.is_empty() {
                err.push_str(&format!(
                    "suggested (from .opencode -> {target}): learn init {suggestion}\n"
                ));
            }
        }
        err.push_str("usage: learn init <profile>\n");
        eprint!("{err}");
        exit_with(1);
    }
    require_slug("profile name", &profile);

    mkdir_p(&ctx.learn_home);
    ctx.store_home = canon_dir(&ctx.learn_home).unwrap_or_else(|| ctx.learn_home.clone());
    let pdir = format!("{}/{}", ctx.store_home, profile);

    if exists(".learnings") || is_symlink(".learnings") {
        if !is_symlink(".learnings") {
            die(".learnings exists but is not a symlink");
        }
        if is_dir(".learnings") {
            let existing = canon_dir(".learnings").unwrap_or_default();
            if existing != pdir {
                die(&format!(
                    ".learnings is bound to {existing}; remove it explicitly before initializing '{profile}'"
                ));
            }
        }
    }

    if !is_dir(&format!("{}/.git", ctx.store_home)) {
        git::must(git::code(Some(&ctx.store_home), &["init", "-q"], false));
    }
    ensure_store_excludes(&ctx.store_home);
    for d in ["inbox", "candidates", "applied", "rejected"] {
        mkdir_p(&format!("{pdir}/{d}"));
    }
    if !is_file(&format!("{pdir}/INDEX.md")) {
        let _ = fs::write(format!("{pdir}/INDEX.md"), "");
    }

    let markers_path = format!("{}/markers.json", ctx.store_home);
    if !is_file(&markers_path) {
        let seed = format!("{}/markers.personal.default.json", ctx.opencode_dir());
        let content = fs::read(&seed).unwrap_or_else(|_| {
            b"{\"version\":1,\"correction\":{\"strong\":[],\"medium\":[],\"weak\":[]},\"confirmation\":[]}\n"
                .to_vec()
        });
        write_atomic(&ctx.store_home, &markers_path, &content);
    }

    if is_symlink(".learnings") {
        let _ = fs::remove_file(".learnings");
    }
    if symlink(&pdir, ".learnings").is_err() {
        die("cannot create .learnings symlink");
    }
    ensure_global_gitignore();
    println!("initialized: profile '{profile}' at {pdir}");
}

// ---------------------------------------------------------------------------
// inbox / ack
// ---------------------------------------------------------------------------

pub fn cmd_inbox(ctx: &mut Ctx, args: &[String]) {
    let usage =
        "usage: learn inbox [--since <ts>] [--kind k] [--tier t] [--all] [--json] [--limit N]";
    let (mut since, mut kind, mut tier) = (String::new(), String::new(), String::new());
    let (mut all, mut json) = (false, false);
    let mut limit = 0usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--since" => since = val(args, &mut i, "--since"),
            "--kind" => kind = val(args, &mut i, "--kind"),
            "--tier" => tier = val(args, &mut i, "--tier"),
            "--all" => {
                all = true;
                i += 1;
            }
            "--json" => {
                json = true;
                i += 1;
            }
            "--limit" => {
                let v = val(args, &mut i, "--limit");
                let ok = v
                    .bytes()
                    .next()
                    .map(|b| (b'1'..=b'9').contains(&b))
                    .unwrap_or(false)
                    && v.bytes().all(|b| b.is_ascii_digit());
                if !ok {
                    die("--limit must be a positive integer");
                }
                limit = v.parse().unwrap_or(usize::MAX);
            }
            _ => die(usage),
        }
    }
    ctx.resolve_profile();
    let ack = if all { String::new() } else { ctx.read_ack() };
    if json {
        let rows = inbox::stream(
            ctx,
            &Filter {
                compact: false,
                since: &since,
                ack: &ack,
                kind: &kind,
                tier: &tier,
                limit,
            },
        );
        for (_, line) in rows {
            println!("{line}");
        }
    } else {
        let rows = inbox::stream(
            ctx,
            &Filter {
                compact: true,
                since: &since,
                ack: &ack,
                kind: &kind,
                tier: &tier,
                limit,
            },
        );
        if rows.is_empty() {
            println!("(no events)");
        } else {
            let mut maxts = "";
            for (k, display) in &rows {
                println!("{display}");
                if k.as_bytes() > maxts.as_bytes() {
                    maxts = k;
                }
            }
            eprintln!("ack: learn ack --until {maxts}");
        }
    }
}

pub fn cmd_ack(ctx: &mut Ctx, args: &[String]) {
    let mut until = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--until" => until = val(args, &mut i, "--until"),
            _ => die("usage: learn ack [--until <ts>]"),
        }
    }
    ctx.resolve_profile();
    if until.is_empty() {
        until = now_utc();
    }
    let date_only = Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$").unwrap();
    if date_only.is_match(&until) {
        until.push_str("T00:00:00Z");
    }
    let full = Regex::new(
        r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}(:[0-9]{2}(\.[0-9]+)?)?(Z|[+-][0-9]{2}:?[0-9]{2})$",
    )
    .unwrap();
    if !full.is_match(&until) {
        die(&format!(
            "invalid timestamp '{until}': use ISO-8601 with a timezone, e.g. 2026-09-30T12:00:00Z"
        ));
    }
    let dir = format!("{}/inbox", ctx.pdir);
    write_atomic(
        &dir,
        &format!("{dir}/.ack"),
        format!("{until}\n").as_bytes(),
    );
    println!("acked inbox until {until}");
}

// ---------------------------------------------------------------------------
// drafts
// ---------------------------------------------------------------------------

fn draft_load(ctx: &Ctx, path: &str) {
    let readable = is_file(path) && fs::File::open(path).is_ok();
    if !readable {
        die(&format!("cannot read draft file: {path}"));
    }
    // Inside `learn dream` (LEARN_DREAM=1) drafts must live in $STORE_HOME/.drafts/:
    // the CLI runs outside the sandbox, so --from must not become a way to read
    // files the dream agent itself may not read.
    if std::env::var("LEARN_DREAM").as_deref() == Ok("1") {
        let real = canonical_path(path)
            .unwrap_or_else(|_| die(&format!("cannot read draft file: {path}")));
        if !real.starts_with(&format!("{}/.drafts/", ctx.store_home)) {
            die(&format!(
                "inside dream, --from must point into {}/.drafts/: {path}",
                ctx.store_home
            ));
        }
    }
}

fn draft_fill(path: &str, var: &mut String, field: &str) {
    if !var.is_empty() {
        return;
    }
    let prefix = format!("{field}: ");
    if let Some(text) = read_text(path) {
        for l in lines_of(&text) {
            if let Some(rest) = l.strip_prefix(&prefix) {
                *var = rest.to_string();
                return;
            }
        }
    }
}

/// Delete the draft after success, only when it lives in $STORE_HOME/.drafts/.
fn draft_cleanup(ctx: &Ctx, path: &str) {
    if let Ok(real) = canonical_path(path) {
        if real.starts_with(&format!("{}/.drafts/", ctx.store_home)) {
            let _ = fs::remove_file(real);
        }
    }
}

fn default_project() -> String {
    match std::env::var("LEARN_PROJECT") {
        Ok(v) if !v.is_empty() => v,
        _ => basename(&pwd_p()),
    }
}

fn validate_project(p: &str) {
    if p.is_empty() || p.contains(',') || p.contains('[') || p.contains(']') || p.contains('\n') {
        die(&format!(
            "invalid project '{p}': must be non-empty without commas or brackets"
        ));
    }
}

fn check_date(d: &str) {
    if !d.is_empty() && !valid_calendar_date(d) {
        die(&format!("invalid date '{d}': use YYYY-MM-DD"));
    }
}

// ---------------------------------------------------------------------------
// candidate lifecycle
// ---------------------------------------------------------------------------

pub fn cmd_add(ctx: &mut Ctx, args: &[String]) {
    let addusage = "usage: learn add --key <area.symptom> --kind <k> --summary \"<=120\" --evidence \"...\" [--project p] [--behavior \"...\"] [--owner o] [--date YYYY-MM-DD] | --from <file>";
    let (mut key, mut kind, mut summary, mut evidence) =
        (String::new(), String::new(), String::new(), String::new());
    let (mut project, mut behavior, mut owner, mut from, mut date) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--from" => from = val(args, &mut i, "--from"),
            "--date" => date = val(args, &mut i, "--date"),
            "--key" => key = val(args, &mut i, "--key"),
            "--kind" => kind = val(args, &mut i, "--kind"),
            "--summary" => summary = val(args, &mut i, "--summary"),
            "--evidence" => evidence = val(args, &mut i, "--evidence"),
            "--project" => project = val(args, &mut i, "--project"),
            "--behavior" => behavior = val(args, &mut i, "--behavior"),
            "--owner" => owner = val(args, &mut i, "--owner"),
            _ => die(addusage),
        }
    }
    if !from.is_empty() {
        draft_load(ctx, &from);
        draft_fill(&from, &mut key, "key");
        draft_fill(&from, &mut kind, "kind");
        draft_fill(&from, &mut summary, "summary");
        draft_fill(&from, &mut evidence, "evidence");
        draft_fill(&from, &mut project, "project");
        draft_fill(&from, &mut behavior, "behavior");
        draft_fill(&from, &mut owner, "owner");
        draft_fill(&from, &mut date, "date");
        key = oneline(&key);
        kind = oneline(&kind);
        project = oneline(&project);
    }
    date = oneline(&date);
    check_date(&date);
    if key.is_empty() || kind.is_empty() || summary.is_empty() || evidence.is_empty() {
        die(addusage);
    }
    ctx.resolve_profile();
    ctx.validate_key(&key);
    if !in_list(&kind, KINDS) {
        die(&format!("invalid kind '{kind}': use one of: {KINDS}"));
    }
    summary = oneline(&summary);
    let n = summary.chars().count();
    if n > 120 {
        die(&format!("summary is {n} characters (max 120)"));
    }
    if summary.is_empty() {
        die("summary must not be empty");
    }
    evidence = oneline(&evidence);
    if evidence.is_empty() {
        die("evidence must not be empty");
    }
    behavior = oneline(&behavior);
    if behavior.is_empty() {
        behavior = summary.clone();
    }
    owner = oneline(&owner);
    if project.is_empty() {
        project = default_project();
    }
    validate_project(&project);

    match ctx.find_key(&key) {
        "candidates" => die(&format!(
            "candidate '{key}' already exists; use: learn bump {key} --evidence \"...\""
        )),
        "applied" => {
            let o = fm_get(&format!("{}/applied/{key}.md", ctx.pdir), "owner");
            die(&format!(
                "'{key}' is already applied (owner: {}); applied patterns never grow",
                if o.is_empty() { "unknown" } else { &o }
            ))
        }
        "rejected" => {
            let path = format!("{}/rejected/{key}.md", ctx.pdir);
            let reason = fm_get(&path, "reason");
            if reason == "stale" {
                let _ = fs::remove_file(&path);
            } else {
                die(&format!(
                    "'{key}' was rejected (reason: {}); delete rejected/{key}.md to re-add it",
                    if reason.is_empty() {
                        "unknown"
                    } else {
                        &reason
                    }
                ));
            }
        }
        _ => {}
    }

    let day = if date.is_empty() { today() } else { date };
    let mut c = String::new();
    c.push_str("---\n");
    c.push_str(&format!("description: {summary}\n"));
    c.push_str(&format!("pattern-key: {key}\n"));
    c.push_str(&format!("kind: {kind}\n"));
    c.push_str("seen: 1\n");
    c.push_str(&format!("first-seen: {day}\n"));
    c.push_str(&format!("last-seen: {day}\n"));
    c.push_str(&format!("projects: [{project}]\n"));
    if !owner.is_empty() {
        c.push_str(&format!("suggested-owner: {owner}\n"));
    }
    c.push_str("status: candidate\n---\n## Future behavior\n");
    c.push_str(&format!(
        "{behavior}\n\n## Evidence\n- {day} {project}: {evidence}\n"
    ));
    let cdir = format!("{}/candidates", ctx.pdir);
    write_atomic(&cdir, &format!("{cdir}/{key}.md"), c.as_bytes());
    ctx.write_index();
    println!("added candidates/{key}.md");
    if !from.is_empty() {
        draft_cleanup(ctx, &from);
    }
}

fn project_listed(list: &str, p: &str) -> bool {
    let l = list.strip_prefix('[').unwrap_or(list);
    let l = l.strip_suffix(']').unwrap_or(l);
    l.split(',').any(|item| item.trim_matches(is_space) == p)
}

pub fn cmd_bump(ctx: &mut Ctx, args: &[String]) {
    let bumpusage = "usage: learn bump <key> --evidence \"...\" [--project p] [--date YYYY-MM-DD] | --from <file>";
    let (mut key, mut evidence, mut project, mut from, mut date) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--from" => from = val(args, &mut i, "--from"),
            "--date" => date = val(args, &mut i, "--date"),
            "--evidence" => evidence = val(args, &mut i, "--evidence"),
            "--project" => project = val(args, &mut i, "--project"),
            _ if a.starts_with('-') => die(&format!("unknown option: {a}")),
            _ => {
                if !key.is_empty() {
                    die(bumpusage);
                }
                key = a.to_string();
                i += 1;
            }
        }
    }
    if !from.is_empty() {
        draft_load(ctx, &from);
        draft_fill(&from, &mut key, "key");
        draft_fill(&from, &mut evidence, "evidence");
        draft_fill(&from, &mut project, "project");
        draft_fill(&from, &mut date, "date");
        key = oneline(&key);
        project = oneline(&project);
    }
    date = oneline(&date);
    check_date(&date);
    if key.is_empty() || evidence.is_empty() {
        die(bumpusage);
    }
    ctx.resolve_profile();
    if !valid_key(&key) {
        die(&format!("invalid key '{key}'"));
    }
    evidence = oneline(&evidence);
    if evidence.is_empty() {
        die("evidence must not be empty");
    }
    if project.is_empty() {
        project = default_project();
    }
    validate_project(&project);

    match ctx.find_key(&key) {
        "applied" => {
            let p = format!("{}/applied/{key}.md", ctx.pdir);
            let o = fm_get(&p, "owner");
            let r = fm_get(&p, "change-ref");
            die(&format!(
                "'{key}' is already applied (owner: {}, change-ref: {}); applied patterns never grow — if the behavior regressed, fix the owner",
                if o.is_empty() { "unknown" } else { &o },
                if r.is_empty() { "unknown" } else { &r }
            ))
        }
        "rejected" => {
            let reason = fm_get(&format!("{}/rejected/{key}.md", ctx.pdir), "reason");
            die(&format!(
                "'{key}' was rejected (reason: {}); not bumping",
                if reason.is_empty() {
                    "unknown"
                } else {
                    &reason
                }
            ))
        }
        "" => die(&format!(
            "no candidate '{key}' (create it with: learn add --key {key} ...)"
        )),
        _ => {}
    }
    let f = format!("{}/candidates/{key}.md", ctx.pdir);
    let seen_s = fm_get(&f, "seen");
    if !is_uint(&seen_s) {
        die(&format!(
            "candidate {key} has invalid seen: '{seen_s}' (run 'learn doctor')"
        ));
    }
    let seen: u64 = seen_s.parse().unwrap_or(0);
    let day = if date.is_empty() { today() } else { date };
    let mut first = fm_get(&f, "first-seen");
    let mut last = fm_get(&f, "last-seen");
    if !valid_calendar_date(&first) || day < first {
        first = day.clone();
    }
    if !valid_calendar_date(&last) || day > last {
        last = day.clone();
    }
    fm_set(&f, "seen", &(seen + 1).to_string());
    fm_set(&f, "first-seen", &first);
    fm_set(&f, "last-seen", &last);
    let mut projects = fm_get(&f, "projects");
    if !project_listed(&projects, &project) {
        let p = projects.strip_prefix('[').unwrap_or(&projects);
        let p = p.strip_suffix(']').unwrap_or(p);
        projects = if p.is_empty() {
            project.clone()
        } else {
            format!("{p}, {project}")
        };
        fm_set(&f, "projects", &format!("[{projects}]"));
    }
    append_evidence(&f, &format!("- {day} {project}: {evidence}"));
    ctx.write_index();
    println!("bumped {key} (seen: {})", seen + 1);
    if !from.is_empty() {
        draft_cleanup(ctx, &from);
    }
}

pub fn cmd_propose(ctx: &mut Ctx, args: &[String]) {
    let usage = "usage: learn propose <key> [--owner <o>]";
    let (mut key, mut owner) = (String::new(), String::new());
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--owner" => owner = val(args, &mut i, "--owner"),
            _ if a.starts_with('-') => die(&format!("unknown option: {a}")),
            _ => {
                if !key.is_empty() {
                    die(usage);
                }
                key = a.to_string();
                i += 1;
            }
        }
    }
    if key.is_empty() {
        die(usage);
    }
    ctx.resolve_profile();
    if !valid_key(&key) {
        die(&format!("invalid key '{key}'"));
    }
    let f = format!("{}/candidates/{key}.md", ctx.pdir);
    if !is_file(&f) {
        die(&format!("no candidate '{key}'"));
    }
    owner = oneline(&owner);
    fm_set(&f, "status", "proposed");
    fm_set(&f, "proposed-at", &today());
    if !owner.is_empty() {
        fm_set(&f, "suggested-owner", &owner);
    }
    ctx.write_index();
    println!("proposed {key}");
}

pub fn cmd_apply(ctx: &mut Ctx, args: &[String]) {
    let usage = "usage: learn apply <key> --owner <o> --change-ref <r>";
    let (mut key, mut owner, mut rf) = (String::new(), String::new(), String::new());
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--owner" => owner = val(args, &mut i, "--owner"),
            "--change-ref" => rf = val(args, &mut i, "--change-ref"),
            _ if a.starts_with('-') => die(&format!("unknown option: {a}")),
            _ => {
                if !key.is_empty() {
                    die(usage);
                }
                key = a.to_string();
                i += 1;
            }
        }
    }
    if key.is_empty() || owner.is_empty() || rf.is_empty() {
        die(usage);
    }
    ctx.resolve_profile();
    if !valid_key(&key) {
        die(&format!("invalid key '{key}'"));
    }
    owner = oneline(&owner);
    if !valid_owner(&owner) {
        die(&format!(
            "invalid owner '{owner}': use script:<path> | skill:<name> | new-skill:<name> | agents:<repo> | config:<path> | hint:<id>"
        ));
    }
    rf = oneline(&rf);
    match ctx.find_key(&key) {
        "candidates" => {}
        "applied" => die(&format!("'{key}' is already applied")),
        "rejected" => die(&format!("'{key}' was rejected; cannot apply")),
        _ => die(&format!("no candidate '{key}'")),
    }
    let f = format!("{}/candidates/{key}.md", ctx.pdir);
    fm_set(&f, "owner", &owner);
    fm_set(&f, "change-ref", &rf);
    fm_set(&f, "applied-at", &today());
    fm_set(&f, "status", "applied");
    mv(
        Path::new(&f),
        Path::new(&format!("{}/applied/{key}.md", ctx.pdir)),
    );
    ctx.write_index();
    println!("applied {key} -> {owner}");
}

pub fn cmd_reject(ctx: &mut Ctx, args: &[String]) {
    let usage = "usage: learn reject <key> --reason \"...\" | --from <file>";
    let (mut key, mut reason, mut from) = (String::new(), String::new(), String::new());
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--from" => from = val(args, &mut i, "--from"),
            "--reason" => reason = val(args, &mut i, "--reason"),
            _ if a.starts_with('-') => die(&format!("unknown option: {a}")),
            _ => {
                if !key.is_empty() {
                    die(usage);
                }
                key = a.to_string();
                i += 1;
            }
        }
    }
    if !from.is_empty() {
        draft_load(ctx, &from);
        draft_fill(&from, &mut key, "key");
        draft_fill(&from, &mut reason, "reason");
        key = oneline(&key);
    }
    if key.is_empty() || reason.is_empty() {
        die(usage);
    }
    ctx.resolve_profile();
    if !valid_key(&key) {
        die(&format!("invalid key '{key}'"));
    }
    reason = oneline(&reason);
    if reason.is_empty() {
        die("reason must not be empty");
    }
    let f = format!("{}/candidates/{key}.md", ctx.pdir);
    if !is_file(&f) {
        die(&format!("no candidate '{key}'"));
    }
    fm_set(&f, "reason", &reason);
    fm_set(&f, "status", "rejected");
    mv(
        Path::new(&f),
        Path::new(&format!("{}/rejected/{key}.md", ctx.pdir)),
    );
    ctx.write_index();
    println!("rejected {key}");
    if !from.is_empty() {
        draft_cleanup(ctx, &from);
    }
}

pub fn cmd_prune(ctx: &mut Ctx, args: &[String]) {
    let mut days_s = "90".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--days" => days_s = val(args, &mut i, "--days"),
            _ => die("usage: learn prune [--days N]"),
        }
    }
    if !is_uint(&days_s) {
        die("--days must be a non-negative integer");
    }
    let days: i64 = days_s.parse().unwrap_or(i64::MAX / 4);
    ctx.resolve_profile();
    let now_days = days_from_civil(&today());
    let mut moved = 0;
    for f in glob_suffix(&format!("{}/candidates", ctx.pdir), ".md") {
        let base = basename(&f);
        let key = base.strip_suffix(".md").unwrap_or(&base).to_string();
        if fm_get(&f, "seen") != "1" || fm_get(&f, "status") != "candidate" {
            continue;
        }
        let last = fm_get(&f, "last-seen");
        if !valid_calendar_date(&last) {
            warn(&format!("skipping {key}: invalid last-seen '{last}'"));
            continue;
        }
        let age = now_days - days_from_civil(&last);
        if age > days {
            fm_set(&f, "reason", "stale");
            fm_set(&f, "status", "rejected");
            mv(
                Path::new(&f),
                Path::new(&format!("{}/rejected/{key}.md", ctx.pdir)),
            );
            println!("pruned {key} ({age}d idle)");
            moved += 1;
        }
    }
    ctx.write_index();
    println!("pruned {moved} candidate(s)");

    // Acked inbox files: filename date older than (ack date - days) means every event is acked and stale.
    let ack = ctx.read_ack();
    let ack_date: String = ack.chars().take(10).collect();
    let mut removed = 0;
    if valid_calendar_date(&ack_date) {
        let ack_days = days_from_civil(&ack_date);
        for f in glob_suffix(&format!("{}/inbox", ctx.pdir), ".jsonl") {
            let base = basename(&f);
            let stem = base.strip_suffix(".jsonl").unwrap_or(&base);
            let fdate = stem.strip_suffix("-backfill").unwrap_or(stem);
            if !valid_calendar_date(fdate) {
                continue;
            }
            if days_from_civil(fdate) < ack_days - days {
                let _ = fs::remove_file(&f);
                removed += 1;
            }
        }
    }
    println!("removed {removed} acked inbox file(s)");
}

// ---------------------------------------------------------------------------
// status / doctor / grep / index
// ---------------------------------------------------------------------------

pub fn cmd_index(ctx: &mut Ctx, args: &[String]) {
    if !args.is_empty() {
        die("usage: learn index");
    }
    ctx.resolve_profile();
    ctx.write_index();
    let text = read_text(format!("{}/INDEX.md", ctx.pdir)).unwrap_or_default();
    let n = text.split('\n').filter(|l| l.starts_with("- [")).count();
    println!("indexed {} ({n} candidates)", ctx.profile);
}

fn count_md(dir: &str) -> usize {
    glob_suffix(dir, ".md").len()
}

pub fn cmd_status(ctx: &mut Ctx, args: &[String]) {
    if !args.is_empty() {
        die("usage: learn status");
    }
    ctx.resolve_profile();
    let ack = ctx.read_ack();
    let unacked = inbox::stream(
        ctx,
        &Filter {
            compact: false,
            since: "",
            ack: &ack,
            kind: "",
            tier: "",
            limit: 0,
        },
    )
    .len();
    println!("store:      {}", ctx.store_home);
    println!("profile:    {}  -> {}", ctx.profile, ctx.pdir);
    println!(
        "candidates: {}",
        count_md(&format!("{}/candidates", ctx.pdir))
    );
    println!("applied:    {}", count_md(&format!("{}/applied", ctx.pdir)));
    println!(
        "rejected:   {}",
        count_md(&format!("{}/rejected", ctx.pdir))
    );
    println!("inbox:      {unacked} unacked event(s)");
    if let Some(pid) = dream::lock_pid(&ctx.store_home) {
        println!("dream:      running (pid {pid})");
    }
    let (_, top) = git::out(
        Some(&ctx.store_home),
        &["rev-parse", "--show-toplevel"],
        true,
    );
    let top_canon = if top.is_empty() {
        None
    } else {
        canon_dir(&top)
    };
    if top_canon.as_deref() == Some(ctx.store_home.as_str()) {
        let (_, st) = git::out(
            Some(&ctx.store_home),
            &["status", "--porcelain", "--untracked-files=all"],
            false,
        );
        let dirty = count_lines(&st);
        if dirty == 0 {
            println!("git:        clean");
        } else {
            println!("git:        {dirty} uncommitted path(s) — run 'learn backup'");
        }
    } else {
        println!("git:        unavailable (store is not a git repository)");
    }
}

fn dprob(ctx: &mut Ctx, msg: &str) {
    println!("{msg}");
    ctx.doc_problems += 1;
}

fn doctor_dir_checks(ctx: &mut Ctx, d: &str) {
    for f in glob_suffix(&format!("{}/{}", ctx.pdir, d), ".md") {
        let base = basename(&f);
        let key = base.strip_suffix(".md").unwrap_or(&base).to_string();
        let p = entry_problems(&f);
        if !p.is_empty() {
            for line in p {
                dprob(ctx, &line);
            }
            continue;
        }
        let desc = fm_get(&f, "description");
        let n = desc.chars().count();
        if n > 120 {
            dprob(
                ctx,
                &format!("LONG DESCRIPTION ({n} chars > 120): {d}/{base}"),
            );
        }
        if !valid_key(&key) {
            dprob(
                ctx,
                &format!("BAD KEY (expected <area>.<symptom>): {d}/{base}"),
            );
        } else {
            let area = key.split('.').next().unwrap_or("");
            if !in_list(area, AREAS) {
                println!("warning: area '{area}' not in vocabulary: {d}/{base}");
            }
        }
        let status = fm_get(&f, "status");
        match d {
            "candidates" => {
                if !matches!(status.as_str(), "candidate" | "proposed" | "open-question") {
                    dprob(
                        ctx,
                        &format!("BAD STATUS ('{status}' in candidates/): {d}/{base}"),
                    );
                }
            }
            "applied" => {
                if status != "applied" {
                    dprob(
                        ctx,
                        &format!("BAD STATUS ('{status}' in applied/): {d}/{base}"),
                    );
                }
                for v in ["owner", "change-ref", "applied-at"] {
                    if fm_get(&f, v).is_empty() {
                        dprob(ctx, &format!("APPLIED MISSING {v}: {d}/{base}"));
                    }
                }
                let v = fm_get(&f, "owner");
                if !v.is_empty() && !valid_owner(&v) {
                    dprob(ctx, &format!(
                        "BAD OWNER ('{v}'; use script:|skill:|new-skill:|agents:|config:|hint:): {d}/{base}"
                    ));
                }
            }
            "rejected" => {
                if status != "rejected" {
                    dprob(
                        ctx,
                        &format!("BAD STATUS ('{status}' in rejected/): {d}/{base}"),
                    );
                }
                if fm_get(&f, "reason").is_empty() {
                    dprob(ctx, &format!("REJECTED MISSING reason: {d}/{base}"));
                }
            }
            _ => {}
        }
    }
}

fn doctor_profile_checks(ctx: &mut Ctx) {
    for d in ["inbox", "candidates", "applied", "rejected"] {
        if !is_dir(&format!("{}/{}", ctx.pdir, d)) {
            println!("warning: missing directory {d}/ (created on first use)");
        }
    }
    for d in ["candidates", "applied", "rejected"] {
        doctor_dir_checks(ctx, d);
    }
    let idx = format!("{}/INDEX.md", ctx.pdir);
    if !is_file(&idx) {
        dprob(ctx, &format!("MISSING INDEX: {idx} (run 'learn index')"));
    } else {
        let expected = ctx.render_index(true);
        let actual = read_text(&idx).unwrap_or_default();
        if expected.trim_end_matches('\n') != actual.trim_end_matches('\n') {
            dprob(
                ctx,
                "INDEX DRIFT: INDEX.md differs from candidates/ (run 'learn index')",
            );
        }
    }
    for f in glob_suffix(&format!("{}/applied", ctx.pdir), ".md") {
        let base = basename(&f);
        if is_file(&format!("{}/candidates/{base}", ctx.pdir)) {
            dprob(
                ctx,
                &format!(
                    "DUPLICATE KEY (in candidates/ and applied/): {}",
                    base.strip_suffix(".md").unwrap_or(&base)
                ),
            );
        }
    }
}

/// Validate a markers/hints file natively (learn-core.js validateMarkers /
/// validateHints). Returns error lines, empty = ok.
fn validate_file(path: &str, hints: bool) -> Vec<String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => return vec![format!("not valid JSON: {e}")],
    };
    let json: Value = match serde_json::from_slice(&bytes) {
        Ok(j) => j,
        Err(e) => return vec![format!("not valid JSON: {e}")],
    };
    if hints {
        markers::validate_hints(&json)
    } else {
        markers::validate_markers(&json)
    }
}

fn doctor_markers_check(ctx: &mut Ctx) {
    let p = format!("{}/markers.json", ctx.store_home);
    if is_file(&p) {
        for e in validate_file(&p, false) {
            dprob(ctx, &format!("BAD MARKERS: {e}"));
        }
    } else {
        println!("warning: {p} missing (run 'learn init <profile>')");
    }
}

fn doctor_hints_check(ctx: &mut Ctx) {
    let p = format!("{}/hints.json", ctx.store_home);
    if !is_file(&p) {
        return;
    }
    for e in validate_file(&p, true) {
        dprob(ctx, &format!("BAD HINTS: {e}"));
    }
}

pub fn cmd_doctor(ctx: &mut Ctx, args: &[String]) {
    if !args.is_empty() {
        die("usage: learn doctor");
    }
    ctx.doc_problems = 0;
    if !ctx.profile_override.is_empty() {
        let po = ctx.profile_override.clone();
        require_slug("profile name", &po);
        if !is_dir(&ctx.learn_home) {
            let h = ctx.learn_home.clone();
            dprob(ctx, &format!("STORE MISSING: {h}"));
            println!("problems: {}", ctx.doc_problems);
            exit_with(1);
        }
        ctx.load_store();
        let p = format!("{}/{}", ctx.store_home, po);
        if !is_dir(&p) {
            die(&format!("no such profile in the store: {po}"));
        }
        ctx.pdir = canon_dir(&p).unwrap_or(p);
        ctx.profile = po;
        if !ctx.is_in_store(&ctx.pdir) {
            let m = format!(
                "UNSAFE PROFILE DIR: {} (outside {})",
                ctx.pdir, ctx.store_home
            );
            dprob(ctx, &m);
        }
        if ctx.doc_problems == 0 {
            doctor_profile_checks(ctx);
        }
        doctor_markers_check(ctx);
        doctor_hints_check(ctx);
        if ctx.doc_problems > 0 {
            die(&format!("problems found ({})", ctx.doc_problems));
        }
        println!("OK");
        return;
    }
    if !exists(".learnings") && !is_symlink(".learnings") {
        die("no .learnings symlink here (ask the user to run: learn init <profile>)");
    }
    if !is_dir(&ctx.learn_home) {
        let h = ctx.learn_home.clone();
        dprob(ctx, &format!("STORE MISSING: {h}"));
        println!("problems: {}", ctx.doc_problems);
        exit_with(1);
    }
    ctx.load_store();
    if !is_symlink(".learnings") {
        dprob(ctx, "NOT A SYMLINK: .learnings");
    } else {
        let target = fs::read_link(".learnings")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !target.starts_with('/') {
            dprob(
                ctx,
                &format!("UNSAFE SYMLINK: .learnings -> {target} (relative target)"),
            );
        }
        if !is_dir(".learnings") {
            dprob(ctx, &format!("BROKEN: .learnings -> {target}"));
        } else {
            let resolved = canon_dir(".learnings").unwrap_or_default();
            if !ctx.is_in_store(&resolved) {
                let m = format!(
                    "UNSAFE SYMLINK: .learnings -> {target} (outside {})",
                    ctx.store_home
                );
                dprob(ctx, &m);
            } else {
                ctx.pdir = resolved;
                ctx.profile = basename(&target);
                doctor_profile_checks(ctx);
            }
        }
    }
    doctor_markers_check(ctx);
    doctor_hints_check(ctx);
    if ctx.doc_problems > 0 {
        die(&format!("problems found ({})", ctx.doc_problems));
    }
    println!("OK");
}

/// Native search (see grep.rs; exotic options fall back to the system grep).
pub fn cmd_grep(ctx: &mut Ctx, args: &[String]) {
    if args.is_empty() {
        die("usage: learn grep <pattern> [extra grep options]");
    }
    ctx.resolve_profile();
    let pattern = &args[0];
    let rc = crate::grep::run(&ctx.pdir, &args[1..], pattern).unwrap_or_else(|m| die(&m));
    match rc {
        0 => {}
        1 => println!("no matches for '{pattern}'"),
        _ => die(&format!("grep failed (exit {rc})")),
    }
}

// ---------------------------------------------------------------------------
// markers
// ---------------------------------------------------------------------------

fn read_json_strict(path: &str) -> Result<Option<Value>, ()> {
    match fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|_| ()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

fn js_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        _ => true,
    }
}

pub fn cmd_markers(ctx: &mut Ctx, args: &[String]) {
    let sub = args.first().cloned().unwrap_or_default();
    let rest: &[String] = if args.is_empty() { args } else { &args[1..] };
    let default_file = format!("{}/markers.default.json", ctx.opencode_dir());
    ctx.load_store();
    let personal_file = format!("{}/markers.json", ctx.store_home);
    let add_usage = "usage: learn markers add --tier strong|medium|weak|confirmation \"<re>\"";
    match sub.as_str() {
        "list" => {
            if !rest.is_empty() {
                die("usage: learn markers list");
            }
            let d = read_json_strict(&default_file)
                .unwrap_or_else(|_| die("cannot read markers (invalid JSON?)"));
            let p = read_json_strict(&personal_file)
                .unwrap_or_else(|_| die("cannot read markers (invalid JSON?)"));
            let groups: [(&str, &[&str]); 4] = [
                ("correction/strong", &["correction", "strong"]),
                ("correction/medium", &["correction", "medium"]),
                ("correction/weak", &["correction", "weak"]),
                ("confirmation", &["confirmation"]),
            ];
            for (name, path) in groups {
                println!("{name}:");
                for (label, x) in [("default", &d), ("personal", &p)] {
                    let mut cur = x.as_ref();
                    for k in path {
                        cur = cur.and_then(|v| v.get(*k));
                    }
                    if let Some(Value::Array(items)) = cur {
                        for m in items {
                            let s = match m {
                                Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            println!("  [{label}] {s}");
                        }
                    }
                }
            }
        }
        "add" => {
            let (mut tier, mut re) = (String::new(), String::new());
            let mut i = 0;
            while i < rest.len() {
                let a = rest[i].as_str();
                match a {
                    "--tier" => tier = val(rest, &mut i, "--tier"),
                    _ if a.starts_with('-') => die(&format!("unknown option: {a}")),
                    _ => {
                        if !re.is_empty() {
                            die(add_usage);
                        }
                        re = a.to_string();
                        i += 1;
                    }
                }
            }
            if !matches!(tier.as_str(), "strong" | "medium" | "weak" | "confirmation") {
                die(add_usage);
            }
            if re.is_empty() {
                die(add_usage);
            }
            let mtmp = format!("{personal_file}.tmp.{}", std::process::id());
            let _ = fs::remove_file(&mtmp);
            let mut j = match fs::read(&personal_file) {
                Ok(b) => match serde_json::from_slice::<Value>(&b) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("markers.json is not valid JSON: {e}");
                        die("could not add marker");
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({
                    "version": 1,
                    "correction": {"strong": [], "medium": [], "weak": []},
                    "confirmation": []
                }),
                Err(e) => {
                    eprintln!("{e}");
                    die("could not add marker");
                }
            };
            let fail = |m: &str| -> ! {
                eprintln!("{m}");
                die("could not add marker")
            };
            let Some(obj) = j.as_object_mut() else {
                fail("markers.json is not a JSON object")
            };
            if !js_truthy(obj.get("version")) {
                obj.insert("version".into(), json!(1));
            }
            if !js_truthy(obj.get("correction")) {
                obj.insert("correction".into(), json!({}));
            }
            let holder: &mut serde_json::Map<String, Value> = if tier == "confirmation" {
                obj
            } else {
                match obj.get_mut("correction").and_then(|c| c.as_object_mut()) {
                    Some(m) => m,
                    None => fail("markers.json: 'correction' is not an object"),
                }
            };
            if !js_truthy(holder.get(&tier)) {
                holder.insert(tier.clone(), json!([]));
            }
            let Some(arr) = holder.get_mut(&tier).and_then(|a| a.as_array_mut()) else {
                fail("markers.json: tier is not an array")
            };
            if arr.iter().any(|x| x.as_str() == Some(re.as_str())) {
                println!("already present in {tier}");
                return;
            }
            arr.push(Value::String(re.clone()));
            let mut text = serde_json::to_string_pretty(&j).unwrap_or_default();
            text.push('\n');
            if fs::write(&mtmp, text).is_err() {
                let _ = fs::remove_file(&mtmp);
                die("could not add marker");
            }
            let merrs = validate_file(&mtmp, false);
            if !merrs.is_empty() {
                let _ = fs::remove_file(&mtmp);
                die(&format!("could not add marker: {}", merrs.join("\n")));
            }
            mv(Path::new(&mtmp), Path::new(&personal_file));
            println!("added to {tier}: {re}");
        }
        "test" => {
            if rest.len() != 1 {
                die("usage: learn markers test \"<text>\"");
            }
            let m = markers::load_markers(&default_file, &personal_file);
            match markers::match_markers(&rest[0], &m) {
                None => println!("no match"),
                Some(r) => println!("{}\t{}", r.tier, r.marker),
            }
        }
        _ => die("usage: learn markers list | add --tier <t> \"<re>\" | test \"<text>\""),
    }
}

// ---------------------------------------------------------------------------
// backup / install
// ---------------------------------------------------------------------------

/// Counts from the staged diff: (added, bumped, applied, rejected).
fn backup_counts(store: &str) -> (u32, u32, u32, u32) {
    let (_, out) = git::out(
        Some(store),
        &["diff", "--cached", "--name-status", "-M"],
        false,
    );
    let (mut add, mut bump, mut applied, mut rej) = (0, 0, 0, 0);
    for line in out.split('\n') {
        let fields: Vec<&str> = line.split('\t').collect();
        let c = fields[0].chars().next().unwrap_or(' ');
        let p = *fields.last().unwrap_or(&"");
        let parts: Vec<&str> = p.split('/').collect();
        if parts.len() != 3
            || parts[0].is_empty()
            || parts[2].is_empty()
            || !parts[2].ends_with(".md")
        {
            continue;
        }
        match parts[1] {
            "candidates" => match c {
                'A' => add += 1,
                'M' => bump += 1,
                _ => {}
            },
            "applied" if c == 'A' || c == 'R' => applied += 1,
            "rejected" if c == 'A' || c == 'R' => rej += 1,
            _ => {}
        }
    }
    (add, bump, applied, rej)
}

pub fn cmd_backup(ctx: &mut Ctx, args: &[String]) {
    let usage = "usage: learn backup [--push]";
    let push = match args.len() {
        0 => false,
        1 => {
            if args[0] != "--push" {
                die(usage);
            }
            true
        }
        _ => die(usage),
    };
    if !is_dir(&format!("{}/.git", ctx.learn_home)) {
        die(&format!(
            "no store at {} (ask the user to run: learn init <profile>)",
            ctx.learn_home
        ));
    }
    ctx.store_home = canon_dir(&ctx.learn_home).unwrap_or_else(|| ctx.learn_home.clone());
    let store = ctx.store_home.clone();
    if let Some(pid) = dream::lock_pid(&store) {
        println!("dream in progress (pid {pid}) — backup skipped");
        return;
    }
    let mut profile = String::new();
    if is_symlink(".learnings") {
        if let Ok(t) = fs::read_link(".learnings") {
            profile = basename(&t.to_string_lossy());
        }
    }
    git::must(git::code(Some(&store), &["add", "-A"], false));
    let clean = git::code_silent_out(Some(&store), &["diff", "--cached", "--quiet"]) == 0
        && git::code_silent_out(Some(&store), &["rev-parse", "-q", "--verify", "HEAD"]) == 0;
    if clean {
        println!("nothing to back up");
    } else {
        let (add, bump, applied, rej) = backup_counts(&store);
        let subject = if profile.is_empty() {
            "learn: ".to_string()
        } else {
            format!("learn({profile}): ")
        };
        let msg = if add == 0 && bump == 0 && applied == 0 && rej == 0 {
            format!("{subject}inbox update")
        } else {
            let mut m = format!("{subject}+{add} candidates, ~{bump} bumped, →{applied} applied");
            if rej > 0 {
                m.push_str(&format!(", -{rej} rejected"));
            }
            m
        };
        if git::code(Some(&store), &["commit", "-q", "-m", &msg], false) != 0 {
            die(&format!(
                "commit failed in {store} — is git user.name / user.email configured?"
            ));
        }
        println!("backed up: {msg}");
    }
    if push {
        if git::code_silent_out(Some(&store), &["remote", "get-url", "origin"]) != 0 {
            die(&format!(
                "no remote configured (git -C {store} remote add origin <url>)"
            ));
        }
        git::must(git::code(
            Some(&store),
            &["push", "-q", "origin", "HEAD"],
            false,
        ));
    }
}

pub fn cmd_install_opencode(ctx: &mut Ctx, args: &[String]) {
    if !args.is_empty() {
        die("usage: learn install-opencode");
    }
    let source = format!("{}/self-improving.js", ctx.opencode_dir());
    if !is_file(&source) {
        die(&format!("OpenCode plugin not found at {source}"));
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("{}/.config", home_dir()));
    let config_dir = format!("{base}/opencode/plugins");
    let dest = format!("{config_dir}/self-improving.js");
    mkdir_p(&config_dir);
    if is_symlink(&dest) {
        let current = canonical_path(&dest).unwrap_or_default();
        if current == source {
            println!("OpenCode plugin already installed: {dest}");
            return;
        }
        let _ = fs::remove_file(&dest);
    } else if exists(&dest) {
        die(&format!("refusing to overwrite non-symlink: {dest}"));
    }
    if symlink(&source, &dest).is_err() {
        die(&format!("cannot create symlink {dest}"));
    }
    println!("installed OpenCode plugin: {dest} (restart OpenCode to load it)");
}
