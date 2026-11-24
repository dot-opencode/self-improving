//! Install and inspect the per-user nightly dream job.

use crate::store::{home_dir, Ctx};
use crate::util::{die, is_uint, mkdir_p, warn, write_atomic};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};

const SERVICE: &str = "agent-learnings-dream.service";
const TIMER: &str = "agent-learnings-dream.timer";
const LABEL: &str = "io.github.dot-opencode.agent-learnings.dream";
const USAGE: &str =
    "usage: learn schedule install [--at HH:MM] [--model <provider/model>] | uninstall | status";

enum Backend {
    Systemd { service: String, timer: String },
    Launchd { plist: String, domain: String },
}

impl Backend {
    fn new() -> Self {
        let scheduler = env::var("AGENT_LEARNINGS_SCHEDULER").unwrap_or_else(|e| match e {
            env::VarError::NotPresent if cfg!(target_os = "macos") => "launchd".into(),
            env::VarError::NotPresent => "systemd".into(),
            env::VarError::NotUnicode(_) => {
                die("AGENT_LEARNINGS_SCHEDULER must be systemd or launchd")
            }
        });
        match scheduler.as_str() {
            "systemd" => {
                let config = env_nonempty("XDG_CONFIG_HOME")
                    .unwrap_or_else(|| format!("{}/.config", required_home()));
                let dir = format!("{config}/systemd/user");
                Self::Systemd {
                    service: format!("{dir}/{SERVICE}"),
                    timer: format!("{dir}/{TIMER}"),
                }
            }
            "launchd" => Self::Launchd {
                plist: format!("{}/Library/LaunchAgents/{LABEL}.plist", required_home()),
                // SAFETY: getuid takes no arguments and returns the real user ID.
                domain: format!("gui/{}", unsafe { libc::getuid() }),
            },
            _ => die("AGENT_LEARNINGS_SCHEDULER must be systemd or launchd"),
        }
    }

    fn path(&self) -> &str {
        match self {
            Self::Systemd { timer, .. } => timer,
            Self::Launchd { plist, .. } => plist,
        }
    }
}

pub fn cmd_schedule(ctx: &Ctx, args: &[String]) {
    match args.split_first().map(|(cmd, rest)| (cmd.as_str(), rest)) {
        Some(("install", rest)) => {
            let mut time = "05:30";
            let mut model = None;
            let mut flags = rest.chunks_exact(2);
            for pair in &mut flags {
                let val = pair[1].as_str();
                match pair[0].as_str() {
                    "--at" if valid_time(val) => time = val,
                    "--model" if !val.is_empty() && !val.starts_with('-') => model = Some(val),
                    _ => die(USAGE),
                }
            }
            if !flags.remainder().is_empty() {
                die(USAGE);
            }
            install(&Backend::new(), ctx, time, model);
        }
        Some(("uninstall", [])) => uninstall(&Backend::new()),
        Some(("status", [])) => status(&Backend::new()),
        _ => die(USAGE),
    }
}

fn valid_time(time: &str) -> bool {
    time.split_once(':').is_some_and(|(h, m)| {
        h.len() == 2 && m.len() == 2 && is_uint(h) && is_uint(m) && h <= "23" && m <= "59"
    })
}

fn env_nonempty(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.is_empty())
}

fn required_home() -> String {
    let home = home_dir();
    if home.is_empty() {
        die("HOME is not set");
    }
    home
}

fn save(path: &str, contents: &str) {
    let dir = crate::util::dirname(path);
    mkdir_p(&dir);
    write_atomic(&dir, path, contents.as_bytes());
}

fn systemctl() -> Command {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user");
    cmd
}

fn checked(cmd: &mut Command, action: &str) {
    match cmd.status() {
        Ok(status) if status.success() => {}
        Ok(status) => die(&format!("{action} failed: {status}")),
        Err(e) => die(&format!("cannot run {action}: {e}")),
    }
}

fn quiet_success(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn install(backend: &Backend, ctx: &Ctx, time: &str, model: Option<&str>) {
    let mut args = vec![ctx.script_path.as_str(), "dream", "--if-changed"];
    if let Some(model) = model {
        args.extend(["--model", model]);
    }
    let mut vars: Vec<_> = [
        "PATH",
        "LEARN_BIN_PATH",
        "LEARN_SKILL_DIR",
        "AGENT_LEARNINGS_DREAM_MODEL",
    ]
    .into_iter()
    .filter_map(|key| env_nonempty(key).map(|val| (key, val)))
    .collect();
    vars.push(("AGENT_LEARNINGS_HOME", ctx.store_home.clone()));
    match backend {
        Backend::Systemd { service, timer } => {
            let mut unit = String::from("[Unit]\nDescription=Consolidate agent learnings\n\n[Service]\nType=oneshot\nNice=10\n");
            for (key, val) in vars {
                let _ = writeln!(
                    unit,
                    "Environment={}",
                    systemd_quote(&format!("{key}={val}"), false)
                );
            }
            let command = args
                .into_iter()
                .map(|arg| systemd_quote(arg, true))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(unit, "ExecStart={command}");
            save(service, &unit);
            save(timer, &format!("[Unit]\nDescription=Run the nightly agent learnings dream\n\n[Timer]\nOnCalendar=*-*-* {time}:00\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"));
            checked(
                systemctl().arg("daemon-reload"),
                "systemctl --user daemon-reload",
            );
            checked(
                systemctl().args(["enable", "--now", TIMER]),
                "systemctl --user enable --now",
            );
            if let Some(user) = env_nonempty("USER") {
                if Command::new("loginctl")
                    .args(["show-user", &user, "-p", "Linger"])
                    .output()
                    .is_ok_and(|o| {
                        o.status.success()
                            && String::from_utf8_lossy(&o.stdout).trim() == "Linger=no"
                    })
                {
                    println!("The timer runs only while logged in. Missed runs catch up at next login (Persistent=true). Use `loginctl enable-linger` to remove that limit.");
                }
            }
        }
        Backend::Launchd { plist, domain } => {
            let logs = format!("{}/Library/Logs", required_home());
            mkdir_p(&logs);
            let log = xml_escape(&format!("{logs}/agent-learnings-dream.log"));
            let (h, m) = time.split_once(':').unwrap();
            let mut text = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n<key>Label</key><string>{LABEL}</string>\n<key>ProgramArguments</key>\n<array>\n");
            for arg in args {
                let _ = writeln!(text, "<string>{}</string>", xml_escape(arg));
            }
            text.push_str("</array>\n<key>EnvironmentVariables</key>\n<dict>\n");
            for (key, val) in vars {
                let _ = writeln!(
                    text,
                    "<key>{key}</key><string>{}</string>",
                    xml_escape(&val)
                );
            }
            let _ = writeln!(text, "</dict>\n<key>StartCalendarInterval</key>\n<dict>\n<key>Hour</key><integer>{}</integer>\n<key>Minute</key><integer>{}</integer>\n</dict>\n<key>StandardOutPath</key><string>{log}</string>\n<key>StandardErrorPath</key><string>{log}</string>\n<key>Nice</key><integer>10</integer>\n</dict>\n</plist>", h.parse::<u8>().unwrap(), m.parse::<u8>().unwrap());
            save(plist, &text);
            bootout(domain);
            checked(
                Command::new("launchctl").args(["bootstrap", domain, plist]),
                "launchctl bootstrap",
            );
        }
    }
    println!(
        "Installed {} at {time}\nCLI: {}",
        backend.path(),
        ctx.script_path
    );
    println!("Note: re-run `learn schedule install` if this path changes (for example after a plugin update).");
    if ctx.script_path.contains("/.cache/") {
        warn(
            "warning: the embedded CLI path is under /.cache/ and may change after a plugin update",
        );
    }
}

fn bootout(domain: &str) {
    quiet_success(Command::new("launchctl").args(["bootout", &format!("{domain}/{LABEL}")]));
}

fn uninstall(backend: &Backend) {
    let installed = |path| match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => die(&format!("cannot inspect {path}: {e}")),
    };
    let remove = |path| match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => die(&format!("cannot remove {path}: {e}")),
    };
    match backend {
        Backend::Systemd { service, timer }
            if installed(timer.as_str()) || installed(service.as_str()) =>
        {
            quiet_success(systemctl().args(["disable", "--now", TIMER]));
            remove(timer.as_str());
            remove(service.as_str());
            checked(
                systemctl().arg("daemon-reload"),
                "systemctl --user daemon-reload",
            );
        }
        Backend::Launchd { plist, domain } if installed(plist.as_str()) => {
            bootout(domain);
            remove(plist.as_str());
        }
        _ => {
            println!("Nightly dream job is not installed.");
            return;
        }
    }
    println!("Removed {}", backend.path());
}

fn status(backend: &Backend) {
    let (name, file) = match backend {
        Backend::Systemd { .. } => ("systemd", backend.path()),
        Backend::Launchd { .. } => ("launchd", backend.path()),
    };
    println!("Backend: {name}\nFile: {file}");
    match fs::read_to_string(file) {
        Ok(text) => {
            println!("Nightly dream job is installed.");
            match backend {
                Backend::Systemd { service, .. } => {
                    if let Some(time) = line_value(&text, "OnCalendar=*-*-* ") {
                        println!("Time: {}", time.strip_suffix(":00").unwrap_or(time));
                    }
                    if let Ok(unit) = fs::read_to_string(service) {
                        if let Some(cmd) = line_value(&unit, "ExecStart=") {
                            println!("Command: {cmd}");
                            if let Some(path) = systemd_first_arg(cmd) {
                                check_cli(&path);
                            }
                        }
                    }
                }
                Backend::Launchd { .. } => {
                    let field = |key| {
                        xml_value(text.split_once(key)?.1, "integer")?
                            .parse::<u8>()
                            .ok()
                    };
                    if let (Some(h), Some(m)) =
                        (field("<key>Hour</key>"), field("<key>Minute</key>"))
                    {
                        println!("Time: {h:02}:{m:02}");
                    }
                    if let Some(array) = xml_value(&text, "array") {
                        let args: Vec<_> = array
                            .split("<string>")
                            .skip(1)
                            .filter_map(|s| s.split_once("</string>"))
                            .map(|(s, _)| xml_unescape(s))
                            .collect();
                        if let Some(path) = args.first() {
                            println!("Command: {}", args.join(" "));
                            check_cli(path);
                        }
                    }
                }
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            println!("Nightly dream job is not installed.")
        }
        Err(e) => die(&format!("cannot read {file}: {e}")),
    }
    match backend {
        Backend::Systemd { .. } => {
            if let Err(e) = systemctl().args(["list-timers", "--all", TIMER]).status() {
                warn(&format!("cannot run systemctl --user list-timers: {e}"));
            }
        }
        Backend::Launchd { domain, .. } => {
            let loaded = quiet_success(
                Command::new("launchctl").args(["print", &format!("{domain}/{LABEL}")]),
            );
            println!(
                "LaunchAgent: {}",
                if loaded { "loaded" } else { "not loaded" }
            );
        }
    }
}

fn check_cli(path: &str) {
    if !Path::new(path).exists() {
        warn(&format!(
            "warning: embedded CLI path no longer exists: {path}"
        ));
    }
}

fn line_value<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines().find_map(|line| line.strip_prefix(prefix))
}

fn systemd_quote(s: &str, exec: bool) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' if exec => out.push_str("$$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

// Read the first argument of a command written by systemd_quote.
fn systemd_first_arg(cmd: &str) -> Option<String> {
    let mut chars = cmd.strip_prefix('"')?.chars();
    let mut path = String::new();
    while let Some(c) = chars.next() {
        path.push(match c {
            '"' => return Some(path),
            '\\' => match chars.next()? {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                c => c,
            },
            '%' | '$' if chars.clone().next() == Some(c) => chars.next()?,
            c => c,
        });
    }
    None
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('\r', "&#13;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&#13;", "\r")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

fn xml_value<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    text.split_once(&format!("<{tag}>"))?
        .1
        .split_once(&format!("</{tag}>"))
        .map(|(value, _)| value)
}
