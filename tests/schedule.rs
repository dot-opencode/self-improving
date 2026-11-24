mod common;

use common::{exists, rd, wr, Env};
use std::fs;
use std::os::unix::fs::PermissionsExt;

const LABEL: &str = "io.github.dot-opencode.agent-learnings.dream";

fn setup(backend: &str) -> Env {
    let mut e = Env::new();
    let bin = e.root.join("bin");
    e.set("PATH", e.path_with(&bin));
    e.set("USER", "learn-test");
    e.set("AGENT_LEARNINGS_SCHEDULER", backend);
    e.set("SCHEDULE_LOG", e.root.join("calls").to_str().unwrap());
    e.unset("LEARN_CLI_PATH");
    e.unset("LEARN_BIN_PATH");
    for name in ["systemctl", "launchctl", "loginctl"] {
        let stub = bin.join(name);
        wr(
            &stub,
            r#"#!/bin/sh
tool=${0##*/}
{ printf '%s' "$tool"; printf ' <%s>' "$@"; printf '\n'; } >> "$SCHEDULE_LOG"
case "$tool" in
  systemctl)
    [ "$2" != list-timers ] || printf 'timer output\n'
    exit "${TEST_SYSTEMCTL_EXIT:-0}"
    ;;
  launchctl)
    case "$1" in
      bootout) exit "${TEST_BOOTOUT_EXIT:-1}" ;;
      print) exit "${TEST_PRINT_EXIT:-0}" ;;
      *) exit "${TEST_BOOTSTRAP_EXIT:-0}" ;;
    esac
    ;;
  loginctl) printf 'Linger=no\n'; exit "${TEST_LOGINCTL_EXIT:-0}" ;;
esac
"#,
        );
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    }
    e
}

#[test]
fn systemd_install_reinstall_and_status() {
    let mut e = setup("systemd");
    let cli = e.root.join(".cache/cli space %\"\\/learn");
    wr(&cli, "");
    e.set("LEARN_CLI_PATH", cli.to_str().unwrap());
    e.set("AGENT_LEARNINGS_HOME", "store space %");
    e.set("LEARN_BIN_PATH", "/custom $binary");
    e.set("LEARN_SKILL_DIR", "");
    e.set("AGENT_LEARNINGS_DREAM_MODEL", "provider/default");
    e.set("AGENT_LEARNINGS_DREAM_PLUGINS", "provider,@scope/plugin");
    let dir = e.home.join(".config/systemd/user");
    let service = dir.join("agent-learnings-dream.service");
    let timer = dir.join("agent-learnings-dream.timer");
    e.run(&[
        "schedule",
        "install",
        "--at",
        "23:59",
        "--model",
        "provider/model",
    ])
    .ok("install")
    .has("timer path", timer.to_str().unwrap())
    .has("CLI path", cli.to_str().unwrap())
    .has("cache warning", "warning:")
    .has("linger note", "loginctl enable-linger");

    let unit = rd(&service);
    let escaped = format!(r#"{}/.cache/cli space %%\"\\/learn"#, e.root.display());
    assert!(unit.contains(&format!(
        r#"ExecStart="{escaped}" "dream" "--if-changed" "--model" "provider/model""#
    )));
    assert!(unit.contains(&format!(
        "Environment=\"AGENT_LEARNINGS_HOME={}/store space %%\"",
        e.proj.display()
    )));
    for needle in [
        "Type=oneshot",
        "Nice=10",
        "Environment=\"PATH=",
        "Environment=\"LEARN_BIN_PATH=/custom $binary\"",
        "Environment=\"AGENT_LEARNINGS_DREAM_MODEL=provider/default\"",
        "Environment=\"AGENT_LEARNINGS_DREAM_PLUGINS=provider,@scope/plugin\"",
    ] {
        assert!(unit.contains(needle), "missing {needle:?} in {unit}");
    }
    assert!(!unit.contains("LEARN_SKILL_DIR"));
    assert!(!unit.contains("TimeoutStartSec"));
    let text = rd(&timer);
    assert!(text.contains("OnCalendar=*-*-* 23:59:00"));
    assert!(text.contains("Persistent=true"));
    assert!(text.contains("WantedBy=timers.target"));
    assert_eq!(rd(e.root.join("calls")), "systemctl <--user> <daemon-reload>\nsystemctl <--user> <enable> <--now> <agent-learnings-dream.timer>\nloginctl <show-user> <learn-test> <-p> <Linger>\n");

    e.run(&["schedule", "status"])
        .ok("status")
        .has("backend", "systemd")
        .has("time", "Time: 23:59")
        .has("command", "\"dream\" \"--if-changed\"")
        .lacks("CLI still exists", "no longer exists");
    fs::remove_file(&cli).unwrap();
    e.run(&["schedule", "status"])
        .ok("status after removal")
        .has(
            "stale CLI path",
            &format!("no longer exists: {}", cli.display()),
        );

    e.run(&["schedule", "install"]).ok("reinstall");
    assert!(rd(timer).contains("OnCalendar=*-*-* 05:30:00"));
    assert!(!rd(service).contains("\"--model\""));
}

#[test]
fn rejects_bad_time_and_backend() {
    let e = setup("systemd");
    for time in [
        "24:00", "05:60", "5:00", "05:0", "05:00:00", "+5:00", "00:−1",
    ] {
        e.run(&["schedule", "install", "--at", time])
            .die(time, "usage:");
    }
    e.run(&["schedule", "install", "--at"])
        .die("missing time", "usage:");
    e.cmd(&["schedule", "status"])
        .env("AGENT_LEARNINGS_SCHEDULER", "cron")
        .run()
        .die("unknown backend", "must be systemd or launchd");
    assert!(!exists(e.root.join("calls")));
}

#[test]
fn systemd_uninstall_and_not_installed() {
    let e = setup("systemd");
    let dir = e.home.join(".config/systemd/user");
    e.run(&["schedule", "install", "--at", "00:00"])
        .ok("install");
    assert!(rd(dir.join("agent-learnings-dream.timer")).contains("OnCalendar=*-*-* 00:00:00"));
    let calls = e.root.join("calls");
    wr(&calls, "");
    e.run(&["schedule", "uninstall"]).ok("uninstall");
    assert!(!exists(dir.join("agent-learnings-dream.service")));
    assert!(!exists(dir.join("agent-learnings-dream.timer")));
    assert_eq!(rd(&calls), "systemctl <--user> <disable> <--now> <agent-learnings-dream.timer>\nsystemctl <--user> <daemon-reload>\n");
    e.run(&["schedule", "uninstall"])
        .ok("uninstall again")
        .has("absent", "not installed");
    e.run(&["schedule", "status"])
        .ok("status")
        .has("absent", "not installed")
        .has("passed through", "timer output");
    assert!(rd(calls)
        .ends_with("systemctl <--user> <list-timers> <--all> <agent-learnings-dream.timer>\n"));
}

#[test]
fn launchd_install_status_and_uninstall() {
    let mut e = setup("launchd");
    let cli = e.root.join("launch & <learn>");
    wr(&cli, "");
    e.set("LEARN_CLI_PATH", cli.to_str().unwrap());
    e.set("LEARN_SKILL_DIR", "/skill \"path\"");
    let plist = e.home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
    e.run(&["schedule", "install", "--at", "07:04"])
        .ok("install");
    let text = rd(&plist);
    for needle in [
        "<key>Hour</key><integer>7</integer>",
        "<key>Minute</key><integer>4</integer>",
        "launch &amp; &lt;learn&gt;",
        "<string>--if-changed</string>",
        "<key>LEARN_SKILL_DIR</key><string>/skill &quot;path&quot;</string>",
        "Library/Logs/agent-learnings-dream.log",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in {text}");
    }
    assert!(e.home.join("Library/Logs").is_dir());
    // SAFETY: getuid takes no arguments and returns the real user ID.
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    let calls = e.root.join("calls");
    let bootout = format!("launchctl <bootout> <{domain}/{LABEL}>\n");
    assert_eq!(
        rd(&calls),
        format!(
            "{bootout}launchctl <bootstrap> <{domain}> <{}>\n",
            plist.display()
        )
    );
    e.run(&["schedule", "status"])
        .ok("loaded status")
        .has("time", "Time: 07:04")
        .has("launchctl exit 0", "LaunchAgent: loaded")
        .lacks("CLI still exists", "no longer exists");
    e.cmd(&["schedule", "status"])
        .env("TEST_PRINT_EXIT", "1")
        .run()
        .ok("unloaded status")
        .has("launchctl exit 1", "LaunchAgent: not loaded");
    assert!(rd(&calls).ends_with(&format!("launchctl <print> <{domain}/{LABEL}>\n")));
    e.run(&["schedule", "uninstall"]).ok("uninstall");
    assert!(!exists(plist));
    assert!(rd(calls).ends_with(&bootout));
}

#[test]
fn systemctl_failure_is_reported() {
    let e = setup("systemd");
    e.cmd(&["schedule", "install"])
        .env("TEST_SYSTEMCTL_EXIT", "7")
        .run()
        .die("reload failure", "systemctl --user daemon-reload failed");
    assert_eq!(
        rd(e.root.join("calls")),
        "systemctl <--user> <daemon-reload>\n"
    );
}
