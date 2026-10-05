# Owner selection

Choose the **narrowest** owner that fixes the behavior. Widen only when evidence
spans more repos or profiles.

| Situation | Owner | Layer |
|---|---|---|
| Mechanically checkable | script / hook / lint | repo |
| A recognizable tool error message | plugin hint | `hint:<id>`, in `opencode/hints.default.json` (generic) or `~/.agent-learnings/hints.json` (user-specific) |
| A shell or tool default setting | config file | `config:<path>`, for example `~/.zshenv` |
| An existing skill failed | patch that skill | wherever it lives |
| Repeated procedure, 1 repo | new skill | `<repo>/.agents/skills` |
| Repeats in ≥2 repos of a profile | new skill | `.agents-configs/common/<profile>/skills` |
| Profile/domain independent | new skill | `~/.agents/skills` |
| Short rule, 1 repo | **suggest** edit to `<repo>/AGENTS.md` | repo |
| Short rule, cross-repo | skill | common / global |

## Where skills live (user's setup)

- `~/.agents/skills` — global; visible everywhere. `npx skills` manages it and may overwrite local edits.
- `.agents-configs/common/<profile>/skills` — profile-bound (`home`, `work`); `config.sh` symlinks it into each project of that profile. Preferred place for cross-repo lessons and for forks of global skills.
- `<repo>/.agents/skills` — repo-local, committed with the repo.
- `AGENTS.md` — only a *suggestion* for one repo. Never a profile-level `AGENTS.md`.

## Deriving the layer

1. Read `projects:` in the candidate frontmatter.
2. For each project, check `readlink <project>/.opencode` → `…/opencode/<profile>`. That gives the project's profile.
3. One project → repo layer. Several projects, same profile → `common/<profile>`. Projects with different profiles, or the lesson has nothing to do with the profile's domain → `~/.agents/skills`.
4. A project without `.opencode` symlink has no profile: stay at repo layer, or ask the user.

## Worked examples

### 1. Script / check

Candidate `test.wrong-cwd`, `seen: 3`, one project: pytest run from a subdirectory
fails to find `conftest.py`. Mechanically checkable → owner `script:scripts/test.sh`
(wrapper that `cd`s to the repo root), layer repo. Proof: running the wrapper from a
subdir passes. Not a skill: a script cannot be forgotten.

### 2. Patch an existing skill

Candidate `git.commit-without-scope`, `seen: 2` in two projects. The `commit`
skill already exists but omits the scope rule. Owner `skill:commit`, layer =
wherever it lives. If it is in `~/.agents/skills`, warn that `npx skills` may
overwrite the patch and prefer a fork in `.agents-configs/common/<profile>/skills`.

### 3. New repo-local skill

Candidate `infra.migration-order`, `seen: 3`, one project: a multi-step DB migration
procedure the agent keeps getting wrong. Repeated procedure, one repo → new skill
`<repo>/.agents/skills/db-migrations/`, created via `writing-skills`. Write
boundary: that directory only.

### 4. New `common/<profile>` skill

Candidate `deps.lockfile-regenerate`, `seen: 2`, `projects: [a, b]`, both with
`.opencode -> …/opencode/work`. Same procedure in ≥2 repos of one profile → new skill
in `.agents-configs/common/work/skills/lockfile-hygiene/`. `config.sh` will symlink
it into both projects. Not `~/.agents/skills`: it is specific to the work stack.

### 5. AGENTS.md suggestion

Candidate `lang.answer-in-russian`, `seen: 3`, one project: short rule, one repo, no
procedure. Owner `agents:<repo>` — **suggest** a one-line addition to that repo's
`AGENTS.md`, show the diff, and let the user decide. If the same rule shows up across
repos, switch to a skill in `common/<profile>` or `~/.agents/skills`; never write a
profile-level `AGENTS.md`.

### 6. Config file

Candidate `shell.zsh-glob-nomatch`, `seen: 3`, several projects: unquoted globs such
as `ls *.log` fail in zsh with `no matches found`. A shell default, not a skill
matter → owner `config:~/.zshenv` (add `setopt no_nomatch`). Proof: an unquoted glob
with no match no longer aborts the command.

### 7. Plugin hint

Candidate `edit.stale-patch`, `seen: 3`: patches fail with `Failed to find expected
lines` because the agent patches from memory. A recognizable tool error message →
owner `hint:patch-reread` in `opencode/hints.default.json` (generic; use
`~/.agent-learnings/hints.json` for user-specific). The hint is appended to the error
the model sees. Preferred over a skill: it fires exactly when the error happens.
