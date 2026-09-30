---
name: self-improving
description: Turn repeated agent mistakes and user corrections into a durable behavior change (script, skill patch, new skill, or AGENTS.md suggestion). Use after a user correction ("нет, я про другое", "no, stop", "I said…"), after a repeated tool failure, on "запомни урок", "разбери inbox", "/learn", "learn from this", and at a natural pause (end of a task) when the context contains a `<self-improving>` block with unprocessed events > 0. Never mid-task.
license: MIT
---

# Self-Improving

Learnings are raw material for **behavior change**, not knowledge. A plugin
captures signals into `.learnings/inbox`; you triage them, count repeats by a
stable key, and — only after the user approves — promote a repeated lesson into
exactly one durable owner. Once `applied`, the lesson lives in that owner, not in
context.

Facts about a project ("service X uses port 5432") belong to `agent-memory`, not here.

No `./.learnings` symlink → this project is not set up. Work without it; you may
suggest `learn init <profile>` to the user, but never run it yourself.

## When

- The user corrected you ("нет", "не так", "я же просил", "no, stop", "I said…").
- The same tool failure happened again in this session or across sessions.
- The user says "запомни урок", "разбери inbox", "/learn", "learn from this".
- A large task just finished — do a short triage while context is fresh.
- Your context contains `<self-improving>` with `N unprocessed events` (N > 0) **and** you are at a natural pause. Do not interrupt a task to triage.

## Never

- Never run `learn init`. Suggest it; the user decides.
- Never edit a skill, script, config, or `AGENTS.md` without an explicit approve of the proposal.
- Never create or edit a profile-level `AGENTS.md`. `AGENTS.md` is only ever a *suggestion* for one repo.
- Never `cat` or grep inbox files directly. Read them with `learn inbox`.
- Never put profanity, personal phrasing, secrets, transcripts, or private paths into candidates, rules, or skills. Raw wording stays in the inbox only.
- Never promote on one occurrence unless the user said "всегда / никогда / запомни / always / never / remember".
- Never treat repetition as truth. Check the lesson is correct before proposing it.
- Never change more than one owner per apply.
- Never grow an `applied` candidate — `learn bump` refuses it; read its owner instead.

## Read

If a `<self-improving>` block is already in your context (the OpenCode plugin
injects it), do not re-read `INDEX.md`. It has:

- one line per candidate: `- [key](candidates/key.md) — kind ×seen: description`;
- `N unprocessed events in .learnings/inbox`;
- a reminder that triage is done by this skill.

Otherwise run `learn status` and read `.learnings/INDEX.md` by path (search
tools do not see `.learnings/`; it is a gitignored symlink).

Read events with `learn inbox` (compact, one line per event; only events after the
last `learn ack`). Useful flags: `--since <ISO>`, `--kind failure|correction|confirmation`,
`--tier strong|medium|weak`, `--all`, `--json`.

## Triage

For each inbox event decide: **noise**, `learn bump <key>`, `learn add …`, or **nothing**.

Keep an event only if all hold:

1. It is about the agent's **behavior** — a project fact instead → suggest `agent-memory`.
2. It is **not derivable** from code, docs, or existing skills.
3. It is **not a one-off** ("сейчас сделай так", a task-specific instruction).
4. The plugin does not capture confirmations. Record one yourself (`--kind confirmation`) only when the user confirmed an approach you yourself considered risky — so you do not drift away from it later.

Then:

- Pick a `pattern-key` = `area.symptom` (kebab-case). Area vocabulary and examples:
  `references/pattern-keys.md`. Run `learn grep <symptom>` first to find an existing key.
- Existing key → `learn bump <key> --evidence "…" [--project p]`. New → `learn add --key … --kind … --summary "<≤120>" --evidence "…"`.
- Write evidence as a redacted one-liner: what happened and what the user wanted. No quotes with profanity, no secrets.
- After processing the batch, run the `learn ack --until <ts>` command that `learn inbox` printed as its trailer, so events that arrived meanwhile stay unprocessed.

## Promotion threshold

Move a candidate to `proposed` (`learn propose <key>`) when any holds:

- `seen ≥ 3` in one project;
- `seen ≥ 2` across ≥ 2 projects;
- the user explicitly said "всегда / никогда / запомни / always / never / remember".

Below the threshold the terminal state is `candidate(seen=N)`. Do not propose early.

## Owner selection

Choose the narrowest owner that fixes the behavior:

| Situation | Owner | Layer |
|---|---|---|
| Mechanically checkable | script / hook / lint | repo |
| An existing skill failed | patch that skill | wherever it lives |
| Repeated procedure, 1 repo | new skill | `<repo>/.agents/skills` |
| Repeats in ≥2 repos of a profile | new skill | `.agents-configs/common/<profile>/skills` |
| Profile/domain independent | new skill | `~/.agents/skills` |
| Short rule, 1 repo | **suggest** edit to `<repo>/AGENTS.md` | repo |
| Short rule, cross-repo | skill | common / global |

The user's setup: skills live in `~/.agents/skills` (global), in
`.agents-configs/common/<profile>/skills` (profile-bound; `config.sh` symlinks them
into projects), and in `<repo>/.agents/skills` (repo-local). Derive the layer from
the candidate's `projects:` and from whether each project's
`.opencode -> …/opencode/<profile>` points at the same profile. Worked examples:
`references/owner-selection.md`.

## Start Packet / Proposal

Before touching any file, show the user and wait for approve:

```markdown
## Proposal: <pattern-key> (seen=N, projects: a, b)
- **Future behavior:** imperative, 1–3 lines.
- **Owner:** script:<path> | skill:<name> | new-skill:<name> | agents:<repo> (suggestion)
- **Layer:** repo | <repo>/.agents/skills | common/<profile> | ~/.agents/skills
- **Write boundary:** exact files that will change (and nothing else).
- **Change:** unified diff, or skeleton of the new skill (name, description, key rules).
- **Stays local:** what remains in inbox/candidate only (raw wording, project details).
- **Proof:** how to check next time that the behavior changed.
```

If the user rejects: `learn reject <key> --reason "…"`. If they ask for changes, revise and show again.

## Apply

After approve, change **exactly one owner**.

- New skill → create it through the `writing-skills` skill. Keep it small; put only the behavior, not the evidence.
- Patching a skill in `~/.agents/skills`: warn the user that `npx skills` may overwrite it later; prefer a fork under `.agents-configs/common/<profile>/skills`.
- `AGENTS.md` is never edited without approve, and only for the repo named in the proposal.
- Record it: `learn apply <key> --owner <script:path|skill:name|new-skill:name|agents:repo> --change-ref <commit|path>`.
- Then run `learn backup`.

## Terminal states

End every triage or proposal by stating **one** of these explicitly per item:

- `applied` — owner changed and recorded.
- `candidate(seen=N)` — kept, below threshold.
- `rejected` — with reason.
- `open-question` — say what is missing (evidence, owner, user decision).
- `no-delta` — nothing worth recording.

## Marker self-learning

Correction markers (regexes for RU/EN phrases) drive the plugin's capture. If
triage confirms a real correction that **no marker matched**, propose a marker:

1. Show the exact regex and tier (`strong|medium|weak|confirmation`).
2. After approve: `learn markers add --tier <t> "<regex>"` (writes personal `~/.agent-learnings/markers.json`).
3. Check with `learn markers test "<text>"`.

Never put profanity or personal phrasing into skills or rules. Markers are personal, local data.

## Relationship to agent-memory

- **Facts** about a project or environment → `agent-memory`. If a triage item is a
  project fact, do not `learn add` it; suggest recording it in memory.
- **Behavior** the agent should change → here.
- After `applied`, the lesson lives in its owner. Do not also copy it into memory.

## CLI cheat sheet

`learn` from PATH, otherwise `<this skill's directory>/bin/learn`.

```
learn init <profile>                 # user-only; never run yourself
learn status | doctor | index | grep <pattern>
learn inbox [--since ts] [--kind k] [--tier t] [--all] [--json]
learn ack [--until ts]
learn add --key <k> --kind <k> --summary "<≤120>" --evidence "<…>" [--project <p>]
learn bump <key> --evidence "<…>" [--project <p>]
learn propose <key> [--owner <o>]
learn apply <key> --owner <o> --change-ref <ref>
learn reject <key> --reason "<…>"
learn prune [--days 90]              # seen=1 and stale → rejected/
learn markers list | add --tier <t> "<re>" | test "<text>"
learn backup [--push] | install-opencode | --version
```
