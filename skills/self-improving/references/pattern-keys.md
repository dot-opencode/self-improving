# Pattern keys

`pattern-key` = `area.symptom`, kebab-case, matching `^[a-z]+\.[a-z0-9-]+$`.
The key is the dedupe unit: `learn bump <key>` instead of a new file. Run
`learn grep <symptom>` before `learn add`.

`area` comes from a controlled vocabulary (unknown areas warn in `learn add` and
`learn doctor`). `symptom` is free but short (2–4 words), describes what went
wrong, not the fix.

| area | Definition | Example keys |
|---|---|---|
| `test` | Running, writing, or interpreting tests | `test.wrong-cwd`, `test.skipped-failing` |
| `build` | Compiling, bundling, CI build steps | `build.stale-cache`, `build.wrong-target` |
| `deps` | Installing, upgrading, locking dependencies | `deps.lockfile-not-updated`, `deps.wrong-package-manager` |
| `git` | Commits, branches, history, PR hygiene | `git.commit-without-scope`, `git.force-push` |
| `shell` | Shell commands, quoting, paths, cwd | `shell.unquoted-path`, `shell.long-running-foreground` |
| `edit` | File edits and patches | `edit.old-string-not-found`, `edit.rewrote-whole-file` |
| `scope` | Doing more or less than asked | `scope.unrequested-refactor`, `scope.touched-unrelated-files` |
| `spec` | Misreading requirements or design docs | `spec.ignored-constraint`, `spec.assumed-requirement` |
| `ui` | Frontend, styling, visual output | `ui.wrong-component-library`, `ui.ignored-design-tokens` |
| `lang` | Language of replies, code comments, docs | `lang.answer-in-russian`, `lang.english-commit-messages` |
| `verbosity` | Too long, too short, over-explaining | `verbosity.too-long-summary`, `verbosity.missing-detail` |
| `tool` | Wrong or misused tool/skill/agent | `tool.grep-instead-of-read`, `tool.skipped-skill` |
| `env` | Local environment, versions, config, credentials handling | `env.wrong-node-version`, `env.missing-dotenv` |
| `infra` | Deployment, cloud, databases, infrastructure procedures | `infra.migration-order`, `infra.wrong-cluster-context` |
