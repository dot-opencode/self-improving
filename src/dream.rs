//! `learn dream`: read-only report, config rendering, lock handling and the
//! sandboxed OpenCode run.

use crate::cmds::val;
use crate::git;
use crate::inbox::{self, Filter};
use crate::store::*;
use crate::util::*;
use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// Small RU + EN words dropped from the similarity tokenizer; they carry no
/// topical signal and would otherwise inflate overlap scores.
const DREAM_STOPWORDS: &str = "the and for with that this from are was not use uses used when into only via per its has have all any but can must should does never always для при как что это или так все его она они без под над про есть быть надо нужно если чтобы когда только уже еще вот";

// ---------------------------------------------------------------------------
// profiles / similarity
// ---------------------------------------------------------------------------

/// Profiles of the store (sorted): valid-slug directories that hold
/// inbox/candidates/applied/rejected. Dot entries and anything whose canonical
/// path leaves the store are skipped.
fn profiles(ctx: &Ctx, only: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(rd) = fs::read_dir(&ctx.store_home) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            names.push(name);
        }
    }
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let d = format!("{}/{}", ctx.store_home, name);
        if !is_dir(&d) || !valid_slug(&name) {
            continue;
        }
        if !only.is_empty() && only != name {
            continue;
        }
        if !["inbox", "candidates", "applied", "rejected"]
            .iter()
            .any(|s| is_dir(&format!("{d}/{s}")))
        {
            continue;
        }
        let Some(canon) = canon_dir(&d) else { continue };
        if !ctx.is_in_store(&canon) {
            continue;
        }
        out.push(name);
    }
    out
}

/// Lowercased words (byte-wise, C locale): anything that is not [a-z0-9] or a
/// byte >= 0x80 separates words, so Cyrillic words stay whole; words under 3
/// characters and stopwords are dropped; duplicates collapse (order kept).
fn tokenize(text: &str) -> Vec<Vec<u8>> {
    let stop: BTreeSet<&[u8]> = DREAM_STOPWORDS.split(' ').map(|w| w.as_bytes()).collect();
    let lowered: Vec<u8> = text
        .bytes()
        .map(|b| if b.is_ascii_uppercase() { b + 32 } else { b })
        .collect();
    let mut words: Vec<Vec<u8>> = Vec::new();
    for w in lowered.split(|b| !(b.is_ascii_lowercase() || b.is_ascii_digit() || *b >= 0x80)) {
        if w.is_empty() {
            continue;
        }
        let clen = w.iter().filter(|b| !(0x80..=0xBF).contains(*b)).count();
        if clen < 3 || stop.contains(w) || words.iter().any(|x| x == w) {
            continue;
        }
        words.push(w.to_vec());
    }
    words
}

/// Rows are sorted `profile\tkey\ttext`. A pair is reported when it shares >= 2
/// words and |shared| / min(|A|,|B|) >= 0.6; different profiles never compare.
fn pairs(rows: &[(String, String, String)]) -> Vec<String> {
    let toks: Vec<Vec<Vec<u8>>> = rows.iter().map(|r| tokenize(&r.2)).collect();
    let mut out = Vec::new();
    for a in 0..rows.len() {
        for b in a + 1..rows.len() {
            if rows[a].0 != rows[b].0 {
                continue;
            }
            let shared: BTreeSet<&Vec<u8>> =
                toks[a].iter().filter(|w| toks[b].contains(w)).collect();
            let minlen = toks[a].len().min(toks[b].len());
            let n = shared.len();
            if !(n >= 2 && minlen > 0 && (n as f64 / minlen as f64) >= 0.6) {
                continue;
            }
            let list: Vec<String> = shared
                .iter()
                .map(|w| String::from_utf8_lossy(w).into_owned())
                .collect();
            out.push(format!(
                "  [{}] candidates/{}.md <-> candidates/{}.md (shared: {})",
                rows[a].0,
                rows[a].1,
                rows[b].1,
                list.join(", ")
            ));
        }
    }
    out
}

fn emit(header: &str, body: &[String]) {
    if body.is_empty() {
        return;
    }
    println!("{header}");
    let mut b: Vec<&String> = body.iter().collect();
    b.sort_by(|x, y| x.as_bytes().cmp(y.as_bytes()));
    for l in b {
        println!("{l}");
    }
}

#[derive(Default)]
struct Acc {
    unacked: Vec<String>,
    unacked_n: usize,
    rows: Vec<(String, String, String)>,
    ready: Vec<String>,
    proposed: Vec<String>,
    stale: Vec<String>,
    unusable: Vec<String>,
    hygiene: Vec<String>,
}

fn scan_profile(ctx: &mut Ctx, prof: &str, stale_days: i64, today_days: i64, acc: &mut Acc) {
    let tag = format!("[{prof}]");
    ctx.profile = prof.to_string();
    let p = format!("{}/{}", ctx.store_home, prof);
    ctx.pdir = canon_dir(&p).unwrap_or(p);

    let ack = ctx.read_ack();
    let rows = inbox::stream(
        ctx,
        &Filter {
            compact: true,
            since: "",
            ack: &ack,
            kind: "",
            tier: "",
            limit: 0,
        },
    );
    if !rows.is_empty() {
        // Unacked count uses the arrival key; oldest/newest show event time (ts).
        let mut tss: Vec<String> = rows
            .iter()
            .map(|(_, d)| {
                d.split([' ', '\t'])
                    .find(|s| !s.is_empty())
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        tss.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        acc.unacked_n += rows.len();
        acc.unacked.push(format!(
            "  {tag} {} event(s), oldest {}, newest {}",
            rows.len(),
            tss[0],
            tss[tss.len() - 1]
        ));
    }

    let index_text = read_text(format!("{}/INDEX.md", ctx.pdir));
    for d in ["candidates", "applied", "rejected"] {
        for f in glob_suffix(&format!("{}/{}", ctx.pdir, d), ".md") {
            let base = basename(&f);
            let key = base.strip_suffix(".md").unwrap_or(&base).to_string();
            let probs = entry_problems(&f);
            if !probs.is_empty() {
                let suffix = format!(": {f}");
                let reason: Vec<&str> = probs
                    .iter()
                    .map(|l| l.strip_suffix(suffix.as_str()).unwrap_or(l))
                    .collect();
                acc.unusable
                    .push(format!("  {tag} {d}/{base} — {}", reason.join("; ")));
                continue;
            }
            let desc = fm_get(&f, "description");
            let n = desc.chars().count();
            if n > 120 {
                acc.hygiene
                    .push(format!("  {tag} long description ({n} chars): {d}/{base}"));
            }
            if valid_key(&key) {
                let area = key.split('.').next().unwrap_or("");
                if !in_list(area, AREAS) {
                    acc.hygiene
                        .push(format!("  {tag} unknown area '{area}': {d}/{base}"));
                }
            } else {
                acc.hygiene.push(format!(
                    "  {tag} bad key (expected <area>.<symptom>): {d}/{base}"
                ));
            }
            if d == "applied" && fm_get(&f, "owner").is_empty() {
                acc.hygiene
                    .push(format!("  {tag} applied without owner: {d}/{base}"));
            }
            if d != "candidates" {
                continue;
            }
            if let Some(idx) = &index_text {
                if !idx.contains(&format!("](candidates/{base})")) {
                    acc.unusable
                        .push(format!("  {tag} {d}/{base} — not in INDEX.md"));
                }
            }
            let sym = key.split_once('.').map(|x| x.1).unwrap_or(&key);
            acc.rows.push((
                prof.to_string(),
                key.clone(),
                oneline(&format!("{sym} {desc}")),
            ));

            let seen: u64 = fm_get(&f, "seen").parse().unwrap_or(0);
            let status = fm_get(&f, "status");
            let last = fm_get(&f, "last-seen");
            let projects = fm_get(&f, "projects");
            let plist = projects.strip_prefix('[').unwrap_or(&projects);
            let plist = plist.strip_suffix(']').unwrap_or(plist).to_string();
            let np = plist
                .split(',')
                .filter(|s| !s.trim_matches(is_space).is_empty())
                .count();
            match status.as_str() {
                "candidate" => {
                    let has_proposal = read_text(&f)
                        .map(|t| t.split('\n').any(|l| l.starts_with("## Proposal")))
                        .unwrap_or(false);
                    if (seen >= 3 || (seen >= 2 && np >= 2)) && !has_proposal {
                        let owner = fm_get(&f, "suggested-owner");
                        let mut line = format!(
                            "  {tag} {key} — seen {seen}, projects: {}",
                            if plist.is_empty() { "none" } else { &plist }
                        );
                        if !owner.is_empty() {
                            line.push_str(&format!(", suggested-owner: {owner}"));
                        }
                        acc.ready.push(line);
                    }
                    if seen == 1 {
                        let age = today_days - days_from_civil(&last);
                        if age > stale_days {
                            acc.stale.push(format!(
                                "  {tag} {key} — seen 1, last-seen {last} ({age}d idle)"
                            ));
                        }
                    }
                }
                "proposed" => {
                    let mut since = fm_get(&f, "proposed-at");
                    if !valid_calendar_date(&since) {
                        since = last.clone();
                    }
                    let age = today_days - days_from_civil(&since);
                    if age > 14 {
                        acc.proposed.push(format!(
                            "  {tag} {key} — proposed {since} ({age}d ago), waiting for a user decision"
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    match index_text {
        Some(actual) => {
            let expected = ctx.render_index(true);
            if expected.trim_end_matches('\n') != actual.trim_end_matches('\n') {
                acc.unusable.push(format!(
                    "  {tag} INDEX.md — index drift (differs from candidates/)"
                ));
            }
        }
        None => acc.unusable.push(format!("  {tag} INDEX.md — missing")),
    }
}

fn report(ctx: &mut Ctx, stale_days: i64, only: &str) {
    ctx.load_store();
    let today_days = days_from_civil(&today());
    let profs = profiles(ctx, only);
    if !only.is_empty() && profs.is_empty() {
        die(&format!("no such profile in the store: {only}"));
    }
    let mut acc = Acc::default();
    for p in &profs {
        scan_profile(ctx, p, stale_days, today_days, &mut acc);
    }
    let mut similar = Vec::new();
    if !acc.rows.is_empty() {
        let mut rows = acc.rows.clone();
        rows.sort_by(|a, b| {
            format!("{}\t{}\t{}", a.0, a.1, a.2)
                .as_bytes()
                .cmp(format!("{}\t{}\t{}", b.0, b.1, b.2).as_bytes())
        });
        similar = pairs(&rows);
    }
    emit(
        "unacked inbox (triage, then 'learn ack --profile <p> --until <ts>'):",
        &acc.unacked,
    );
    emit(
        "similar candidates (merge, or confirm they differ):",
        &similar,
    );
    emit(
        "ready for proposal (threshold met, no ## Proposal section):",
        &acc.ready,
    );
    emit(
        "proposed without decision (older than 14 days — needs the user):",
        &acc.proposed,
    );
    emit(
        &format!("stale (seen 1, older than {stale_days} days — reject or keep):"),
        &acc.stale,
    );
    emit(
        "unusable (fix frontmatter / run 'learn index'):",
        &acc.unusable,
    );
    emit("hygiene:", &acc.hygiene);

    let (a, c, e, f, g) = (
        acc.unacked_n,
        similar.len(),
        acc.ready.len(),
        acc.stale.len(),
        acc.unusable.len(),
    );
    let h = acc.proposed.len() + acc.hygiene.len();
    if a + c + e + f + g + h == 0 {
        println!("dream: learnings are tidy — nothing to consolidate");
    } else {
        println!("dream: {a} unacked, {c} similar, {e} ready, {f} stale, {g} unusable");
    }
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn render(template: &str, store: &str, bin: &str, home: &str) -> String {
    let s = template.trim_end_matches('\n');
    let s = s.replace("__STORE__", store);
    let s = s.replace("__LEARN_BIN__", bin);
    s.replace("__HOME__", home)
}

// ---------------------------------------------------------------------------
// lock
// ---------------------------------------------------------------------------

/// PID in `$STORE_HOME/.dream.lock` when that process is alive.
pub fn lock_pid(store: &str) -> Option<u32> {
    let line = first_line_of(Path::new(&format!("{store}/.dream.lock")))?;
    if !is_uint(&line) {
        return None;
    }
    let pid: u32 = line.parse().ok()?;
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    // SAFETY: signal 0 only probes for existence.
    if unsafe { libc::kill(pid as i32, 0) } == 0 {
        Some(pid)
    } else {
        None
    }
}

extern "C" fn on_signal(sig: libc::c_int) {
    if let Some(c) = HELD_LOCK_C.get() {
        // SAFETY: unlink and _exit are async-signal-safe; the CString lives in a static.
        unsafe {
            libc::unlink(c.as_ptr());
            libc::_exit(128 + sig);
        }
    }
    // SAFETY: as above.
    unsafe { libc::_exit(128 + sig) }
}

/// Take the lock atomically (create_new); replace a stale or unreadable one;
/// die when a live dream holds it. Released on every exit path.
fn lock_acquire(store: &str) {
    let lock = format!("{store}/.dream.lock");
    let mut tries = 0;
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{}", std::process::id());
                break;
            }
            Err(_) => {
                if let Some(pid) = lock_pid(store) {
                    die(&format!("another learn dream is running (pid {pid})"));
                }
                tries += 1;
                if tries > 5 {
                    die(&format!("cannot take the dream lock {lock}"));
                }
                let _ = fs::remove_file(&lock);
            }
        }
    }
    let _ = HELD_LOCK.set(Path::new(&lock).to_path_buf());
    if let Ok(c) = CString::new(lock) {
        let _ = HELD_LOCK_C.set(c);
    }
    // SAFETY: installing plain handlers for TERM/INT/HUP.
    unsafe {
        for s in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            let handler: extern "C" fn(libc::c_int) = on_signal;
            libc::signal(s, handler as usize as libc::sighandler_t);
        }
    }
}

/// Stage everything and commit; false when there was nothing to commit.
fn commit(store: &str, msg: &str) -> bool {
    if git::code(Some(store), &["add", "-A"], false) != 0 {
        die(&format!("git add failed in {store}"));
    }
    if git::code_silent_out(Some(store), &["diff", "--cached", "--quiet"]) == 0
        && git::code_silent_out(Some(store), &["rev-parse", "-q", "--verify", "HEAD"]) == 0
    {
        return false;
    }
    if git::code(Some(store), &["commit", "-q", "-m", msg], false) != 0 {
        die(&format!(
            "commit failed in {store} — is git user.name / user.email configured?"
        ));
    }
    true
}

/// Last non-blank line of the agent's output, if it looks like the requested
/// summary ("summary: triaged …" / "triaged …"); empty otherwise.
fn summary_line(text: &str) -> String {
    let mut clean = Vec::new();
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == 0x1b && i + 1 < b.len() && b[i + 1] == b'[' {
            let mut j = i + 2;
            while j < b.len() && (b[j].is_ascii_digit() || b[j] == b';') {
                j += 1;
            }
            if j < b.len() && b[j].is_ascii_alphabetic() {
                i = j + 1;
                continue;
            }
        }
        clean.push(b[i]);
        i += 1;
    }
    let clean = String::from_utf8_lossy(&clean).into_owned();
    let last = clean
        .split('\n')
        .rfind(|l| l.chars().any(|c| !(c == ' ' || c == '\t')))
        .unwrap_or("");
    let mut line = oneline(last);
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("summary:") || lower.starts_with("dream:") {
        line = line.split_once(':').map(|x| x.1).unwrap_or("").to_string();
    } else if !lower.starts_with("triaged ") {
        return String::new();
    }
    let line = oneline(&line);
    line.chars().take(120).collect()
}

fn which(cmd: &str) -> bool {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':').any(|d| {
        let d = if d.is_empty() { "." } else { d };
        let p = format!("{d}/{cmd}");
        fs::metadata(&p)
            .map(|m| {
                use std::os::unix::fs::PermissionsExt;
                m.is_file() && m.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false)
    })
}

pub fn cmd_dream(ctx: &mut Ctx, args: &[String]) {
    let usage = "usage: learn dream [--report [--stale-days N]] [--profile <p>] [--model <provider/model>] [--if-changed] [--print-config]";
    // AGENT_LEARNINGS_DREAM_MODEL sets the default model; --model overrides it.
    let mut model = std::env::var("AGENT_LEARNINGS_DREAM_MODEL").unwrap_or_default();
    let (mut report_f, mut print_config, mut model_flag, mut if_changed) =
        (false, false, false, false);
    let mut stale_days: Option<i64> = None;
    let mut profile = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--report" => {
                report_f = true;
                i += 1;
            }
            "--print-config" => {
                print_config = true;
                i += 1;
            }
            "--if-changed" => {
                if_changed = true;
                i += 1;
            }
            "--stale-days" => {
                if i + 1 >= args.len() {
                    die(usage);
                }
                let v = &args[i + 1];
                if !is_uint(v) {
                    die(usage);
                }
                let n: i64 = v.parse().unwrap_or(i64::MAX / 4);
                if n <= 0 {
                    die(usage);
                }
                stale_days = Some(n);
                i += 2;
            }
            "--profile" => {
                if i + 1 >= args.len() {
                    die(usage);
                }
                profile = args[i + 1].clone();
                i += 2;
            }
            "--model" => {
                if i + 1 >= args.len() || args[i + 1].is_empty() {
                    die(usage);
                }
                model = val(args, &mut i, "--model");
                model_flag = true;
            }
            _ => die(usage),
        }
    }
    if report_f {
        if model_flag || print_config || if_changed {
            die(usage);
        }
    } else if stale_days.is_some() {
        die(usage);
    }
    let stale_days = stale_days.unwrap_or(90);

    ctx.load_store();
    if !profile.is_empty() {
        require_slug("profile name", &profile);
        if !is_dir(&format!("{}/{}", ctx.store_home, profile)) {
            die(&format!("no such profile in the store: {profile}"));
        }
    }
    if report_f {
        report(ctx, stale_days, &profile);
        return;
    }

    let assets = format!("{}/dream", ctx.skill_dir);
    let cfg_path = format!("{assets}/opencode.json");
    let prompt_path = format!("{assets}/prompt.md");
    if !is_file(&cfg_path) || !is_file(&prompt_path) {
        die(&format!("dream assets not found in {assets}"));
    }
    let home = home_dir();
    for bad in [&ctx.store_home, &home, &ctx.script_path] {
        if bad.contains('"') || bad.contains('\\') || bad.contains('\n') {
            die(&format!(
                "cannot run dream: path contains a quote, backslash or newline: {bad}"
            ));
        }
        if bad.contains(['*', '?', '[', ']', '{', '}']) {
            die(&format!(
                "cannot run dream: path contains a glob character (* ? [ ] {{ }}): {bad}"
            ));
        }
    }
    let config = render(
        &read_text(&cfg_path).unwrap_or_default(),
        &ctx.store_home,
        &ctx.script_path,
        &home,
    );
    let mut prompt = render(
        &read_text(&prompt_path).unwrap_or_default(),
        &ctx.store_home,
        &ctx.script_path,
        &home,
    );
    if !profile.is_empty() {
        prompt.push_str(&format!(
            "\n\nProfile scope: {profile} (pass --profile {profile} to every learn command)"
        ));
    }
    if print_config {
        println!("{config}");
        return;
    }

    let store = ctx.store_home.clone();
    if !is_dir(&format!("{store}/.git")) {
        die(&format!(
            "store at {store} is not a git repository (ask the user to run: learn init <profile>)"
        ));
    }
    if !which("opencode") {
        die("opencode not found on PATH");
    }

    ensure_store_excludes(&store);
    lock_acquire(&store);
    let (_, st) = git::out(
        Some(&store),
        &["status", "--porcelain", "--untracked-files=all"],
        false,
    );
    if !st.is_empty() {
        commit(&store, "learn: pre-dream snapshot");
        println!("dream: committed pending store changes first");
    }
    let (_, pre) = git::out(Some(&store), &["rev-parse", "-q", "--verify", "HEAD"], true);

    // --if-changed: skip when no commit touched candidates/inbox/applied/rejected
    // since the sha stored in .dream-last.
    let last_path = format!("{store}/.dream-last");
    if if_changed && !pre.is_empty() && is_file(&last_path) {
        let last = first_line_of(Path::new(&last_path)).unwrap_or_default();
        if !last.is_empty()
            && git::code_silent_out(
                Some(&store),
                &["cat-file", "-e", &format!("{last}^{{commit}}")],
            ) == 0
        {
            let (_, diff) = git::out(
                Some(&store),
                &[
                    "diff",
                    "--name-only",
                    &last,
                    &pre,
                    "--",
                    "*/candidates/*",
                    "*/inbox/*",
                    "*/applied/*",
                    "*/rejected/*",
                ],
                false,
            );
            if diff.is_empty() {
                println!(
                    "dream: nothing changed in candidates/inbox/applied/rejected since the last run — skipping"
                );
                release_lock_if_ours();
                return;
            }
        }
    }

    // The agent writes free text to .drafts/draft.md; do not rely on its write tool creating the dir.
    mkdir_p(&format!("{store}/.drafts"));
    // The unattended session lives in its own OpenCode database (inside .git: never
    // committed), so it never shows up in the user's OpenCode session list.
    let db = format!("{store}/.git/learn-dream.db");
    for s in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{db}{s}"));
    }
    let tmpdir = std::env::var("TMPDIR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/tmp".into());
    let outfile =
        mktemp_in(&tmpdir, "learn-dream.").unwrap_or_else(|_| die("cannot create temporary file"));

    let mut cmd = Command::new("opencode");
    cmd.current_dir(&store)
        .env("LEARN_DREAM", "1")
        .env("AGENT_LEARNINGS_HOME", &store)
        .env("OPENCODE_CONFIG_CONTENT", &config)
        .env("OPENCODE_DB", &db)
        .args(["run", "--standalone", "--agent", "learn-dream"]);
    if !model.is_empty() {
        cmd.args(["--model", &model]);
    }
    cmd.arg(&prompt).stdout(Stdio::piped());
    let _ = std::io::stdout().flush();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            let _ = fs::remove_file(&outfile);
            die("opencode run failed (exit 127); store changes, if any, are left uncommitted for review");
        }
    };
    // tee: copy the agent's stdout to ours and to the capture file.
    let mut captured: Vec<u8> = Vec::new();
    if let Some(mut so) = child.stdout.take() {
        let mut buf = [0u8; 8192];
        let stdout = std::io::stdout();
        loop {
            match so.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    captured.extend_from_slice(&buf[..n]);
                    let mut h = stdout.lock();
                    let _ = h.write_all(&buf[..n]);
                    let _ = h.flush();
                }
            }
        }
    }
    let status = child.wait();
    let rc = match status {
        Ok(s) => s.code().unwrap_or(128 + git::signal_of(&s)),
        Err(_) => 127,
    };
    let _ = fs::write(&outfile, &captured);
    if rc != 0 {
        let _ = fs::remove_file(&outfile);
        die(&format!(
            "opencode run failed (exit {rc}); store changes, if any, are left uncommitted for review"
        ));
    }

    println!("dream: session transcript in {db} (latest run only)");
    let mut summary = summary_line(&String::from_utf8_lossy(&captured));
    let _ = fs::remove_file(&outfile);
    // Leftover drafts (agent scratch files) never outlive the run.
    if let Ok(rd) = fs::read_dir(format!("{store}/.drafts")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && !is_symlink(&p.to_string_lossy()) {
                let _ = fs::remove_dir_all(&p);
            } else {
                let _ = fs::remove_file(&p);
            }
        }
    }
    if summary.is_empty() {
        summary = "consolidated".into();
    }

    // Rebuild every profile's INDEX.md before the final commit.
    for p in profiles(ctx, "") {
        ctx.profile_override = p;
        ctx.resolve_profile();
        ctx.write_index();
    }
    ctx.profile_override.clear();

    // Inbox events the plugin appended during the run get their own commit, so
    // the dream commit holds only the agent's changes, .ack and INDEX.
    let inbox_spec = "*/inbox/*.jsonl";
    if git::code_silent_out(Some(&store), &["rev-parse", "-q", "--verify", "HEAD"]) == 0 {
        let (_, s) = git::out(
            Some(&store),
            &[
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--",
                inbox_spec,
            ],
            false,
        );
        if !s.is_empty() {
            if git::code(Some(&store), &["add", "-A", "--", inbox_spec], false) != 0 {
                die(&format!("git add failed in {store}"));
            }
            if git::code_silent_out(
                Some(&store),
                &["diff", "--cached", "--quiet", "--", inbox_spec],
            ) != 0
                && git::code(
                    Some(&store),
                    &[
                        "commit",
                        "-q",
                        "-m",
                        "learn: inbox events recorded during dream",
                        "--",
                        inbox_spec,
                    ],
                    false,
                ) != 0
            {
                die(&format!(
                    "commit failed in {store} — is git user.name / user.email configured?"
                ));
            }
        }
    }

    let committed = commit(&store, &format!("learn: dream — {summary}"));
    let (rc, head) = git::out(Some(&store), &["rev-parse", "HEAD"], true);
    if git::code_silent_out(Some(&store), &["rev-parse", "-q", "--verify", "HEAD"]) == 0 && rc == 0
    {
        let _ = fs::write(&last_path, format!("{head}\n"));
    }

    if committed {
        println!("dream: committed — {summary}");
        println!("undo: git -C {store} revert HEAD");
    } else {
        println!("dream: no changes");
    }
}
