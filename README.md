# opencode-self-improving

A skill, an OpenCode plugin, and a small Rust CLI (`learn`) that turn repeated
agent mistakes and user corrections into a **durable behavior change** — a
script, a skill patch, a new skill, or (as a suggestion only) an `AGENTS.md` line.

## What and why

- The plugin deterministically captures signals into an inbox: tool failures and
  user corrections/confirmations (matched by RU/EN regex markers).
- The nightly `learn dream` triages the inbox, counts repeats by a stable
  `pattern-key`, and prepares proposals with an explicit owner, layer, write
  boundary, diff, and proof. The plugin injects the list of `proposed` candidates
  into sessions; the `self-improving` skill presents them at a natural pause and
  applies one only after user approval. In-session triage of the inbox happens
  only when you ask for it (say "разбери inbox" or "/learn" in chat; these are plain chat phrases, not registered slash commands).
  The unattended dream run uses its own OpenCode database at
  `<store>/.git/learn-dream.db`, which holds the latest run only, so dream sessions
  never show up in OpenCode's session list. Inspect it with `sqlite3`.
- Once applied, the lesson lives in its owner — not in context — and the candidate
  stops growing.

### Compared to other tools

- **[kurbezz/memory](https://github.com/kurbezz/memory) (agent-memory)** stores *facts*
  ("what is true about this project"), read every session. This project stores
  *behavior* ("what the agent should do differently"): raw material for promotion,
  not context. Facts found during triage are handed off to agent-memory.
- **[zhaono1/agent-playbook](https://github.com/zhaono1/agent-playbook) `self-improving-agent`**
  uses a JSON state machine and an external evals CLI. This project has neither:
  markdown files and a small CLI, in the conventions of `kurbezz/memory`. It borrows the
  Start Packet (owner/proof/write boundary before edits), explicit terminal states,
  the narrow → wide owner hierarchy, and "repetition ≠ truth".

## Install

```bash
npx skills add dot-opencode/self-improving -g -a opencode
learn install-opencode          # symlinks the plugin into ~/.config/opencode/plugins/
cd <project> && learn init <profile>   # e.g. home | work
```

`npx skills add ... -g -a opencode` installs the skill to
`~/.agents/skills/self-improving/` (OpenCode reads that directory directly), so
the CLI lands at `~/.agents/skills/self-improving/bin/learn`. It is not on `PATH`
automatically. Check that it kept its executable bit (`ls -l`; if not,
`chmod +x` it), then link it, e.g.:

```bash
mkdir -p ~/.local/bin
ln -sf ~/.agents/skills/self-improving/bin/learn ~/.local/bin/learn
export PATH="$HOME/.local/bin:$PATH"   # add to your shell profile
```

`bin/learn` is a small POSIX sh bootstrap: on first run it downloads the Rust binary
for your platform from the matching GitHub Release, checks its sha256, and caches it
as `bin/.learn-<version>` (or under `~/.cache/self-improving/` if the skill dir is
read-only). Set `LEARN_BIN_PATH` to use a locally built binary instead.

In a clone of this repo it is `skills/self-improving/bin/learn` (and `bin/learn`).
Run `learn install-opencode` after putting it on `PATH`, or call it by full path.
`learn init` is user-only; the agent never runs it.

## Storage layout

```
~/.agent-learnings/                 # git repo; override with AGENT_LEARNINGS_HOME
  markers.json                      # personal markers (local)
  <profile>/
    inbox/YYYY-MM-DD.jsonl          # raw plugin events (read via `learn inbox`); optional `ats` = arrival time (backfills), the ack/order key when present
    candidates/<pattern-key>.md
    applied/<pattern-key>.md
    rejected/<pattern-key>.md
    INDEX.md                        # generated; candidates only
<project>/.learnings -> ~/.agent-learnings/<profile>   # symlink, globally gitignored
```

A `pattern-key` is `area.symptom` (e.g. `test.wrong-cwd`). Areas:
`test, build, deps, git, shell, edit, scope, spec, ui, lang, verbosity, tool, env, infra`.

## Lifecycle

```
inbox event ──triage──▶ candidate (seen=1)
                          │  learn bump  (seen++)
                          ▼
              threshold: seen≥3 in 1 project | seen≥2 in ≥2 projects | "always/never/remember"
                          ▼
                       proposed ──user approve──▶ applied   (owner + change-ref; never grows)
                          │
                          └──user rejects / stale──▶ rejected (reason)
```

Every triage ends in one explicit state: `applied | candidate(seen=N) | rejected |
open-question | no-delta`. `learn prune` moves `seen=1` candidates that are stale
(default 90 days) to `rejected/` with `reason: stale`.

## Slash commands

The plugin registers two commands. They scan every profile in the store, so they work from any project, with or without a `.learnings` symlink.

- `/learn-check`: posts a short summary of proposals (`status: proposed`) waiting for review, plus candidate and unprocessed-inbox counts per profile. No model call.
- `/learn-review [profile|pattern-key]`: starts a one-by-one review of the proposed candidates (optionally filtered); you approve, request changes or reject each one.

## Markers

Two files with the same schema; the plugin merges them (personal after default):

- `skills/self-improving/opencode/markers.default.json` — shared RU/EN markers.
- `~/.agent-learnings/markers.json` — personal, created by `learn init`, never in this repo.

Tool-error hints (extra text appended to a recognizable tool error the model sees): `skills/self-improving/opencode/hints.default.json`, plus optional personal `~/.agent-learnings/hints.json` (same `id` overrides the default).

Tiers: `correction.{strong,medium,weak}` and `confirmation`. Manage with
`learn markers list | add --tier <t> "<re>" | test "<text>"`. When triage finds a
real correction no marker caught, the skill proposes `learn markers add` (after your approve).

Capture constraints: follow-up messages only, top-level sessions only, length < 400,
no URLs / `/Users/` paths / pasted logs; `weak` markers count only right after an
agent action and for messages < 200 chars.

## Privacy

- The inbox stores only a redacted `head`/`snippet` (≤ 300 chars): tokens, API keys,
  passwords, `Bearer …`, `sk-…`, `ghp_…`, `AKIA…`, `xox…`, and URLs with credentials
  become `[redacted]`. No transcripts, no tool I/O.
- `.learnings` is globally gitignored in projects (as `.learnings`, without a trailing slash: it is a symlink, and git treats symlinks as files); the store is its own git repo
  (`learn backup` commits, `--push` pushes). Keep its remote private.
- Candidates, skills, and rules never contain profanity or personal phrasing.

## Limitations

- macOS and Linux (x86_64, arm64) only; the CLI requires `git`, plus `curl` or `wget` for the first download. Node ≥ 22 is needed only for the OpenCode plugin.
- Symlinks are absolute; moving `~/.agent-learnings` breaks project `.learnings` links (`learn doctor` reports it).
- Targets the OpenCode V2 plugin API, which is in beta: hook names and the shape of `execute.after` events may change. Run with `SELF_IMPROVING_DEBUG=1` to log event types to stderr. The plugin never throws.
- The store assumes a single writer for git backup; concurrent sessions may race on commits.
- Markers are heuristics: expect misses and false positives; triage is the filter.

## Development

```bash
npm test     # cargo test && node --test tests/*.test.js
```

The CLI is a Rust crate at the repo root (`src/`, `tests/*.rs`); the plugin has no npm
dependencies (`node:test` only). Releases: bump `version` in `Cargo.toml` and `VERSION`
in `skills/self-improving/bin/learn`, then push a `v<version>` tag.

## License

MIT © 2026 kurbezz
