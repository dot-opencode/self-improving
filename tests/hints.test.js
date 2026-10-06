import assert from "node:assert/strict"
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

import { applyHint, loadHints, matchHint, validateHints } from "../skills/self-improving/opencode/learn-core.js"

const defaultPath = fileURLToPath(new URL("../skills/self-improving/opencode/hints.default.json", import.meta.url))
const hints = loadHints(defaultPath, undefined)

test("default hints: exactly the shipped ids and valid", () => {
  assert.deepEqual(hints.map((h) => h.id), ["patch-reread", "patch-add-file", "grep-regex", "zsh-status-var", "read-offset-number", "memory-symlink-pathspec"])
  assert.deepEqual(validateHints(JSON.parse(readFileSync(defaultPath, "utf8"))), [])
})

test("error status: hint appended to error.message once", () => {
  const ev = { status: "error", tool: "edit", error: { message: "Could not find oldString in file" } }
  const hint = matchHint(ev, hints)
  assert.equal(hint.id, "patch-reread")
  assert.equal(applyHint(ev, hint), true)
  assert.ok(ev.error.message.startsWith("Could not find oldString in file\nHint: your copy"))
  // No double append.
  assert.equal(applyHint(ev, hint), false)
  assert.equal(ev.error.message.split(hint.text).length, 2)
})

test("error status with string error is replaced", () => {
  const ev = { status: "error", tool: "grep", error: "regex parse error: look-around not supported" }
  const hint = matchHint(ev, hints)
  assert.equal(hint.id, "grep-regex")
  assert.equal(applyHint(ev, hint), true)
  assert.equal(typeof ev.error, "string")
  assert.ok(ev.error.endsWith(hint.text))
})

test("completed shell with exit != 0 gets a content part", () => {
  const ev = {
    status: "completed",
    tool: "shell",
    result: { content: [{ type: "text", text: "zsh: read-only variable: status" }], metadata: { metadata: { exit: 1 } } },
  }
  const hint = matchHint(ev, hints)
  assert.equal(hint.id, "zsh-status-var")
  assert.equal(applyHint(ev, hint), true)
  assert.equal(ev.result.content.length, 2)
  assert.deepEqual(ev.result.content[1], { type: "text", text: hint.text })
  assert.equal(applyHint(ev, hint), false)
  assert.equal(ev.result.content.length, 2)
})

test("no hint on success, exit 0, or non-matching failure", () => {
  const ok = { status: "completed", tool: "shell", result: { content: [{ type: "text", text: "read-only variable: status" }], metadata: { metadata: { exit: 0 } } } }
  assert.equal(matchHint(ok, hints), undefined)
  const okEdit = { status: "completed", tool: "edit", result: { content: [{ type: "text", text: "Could not find oldString" }] } }
  // detectToolFailure flags this as failure (oldString), so it is a legit match.
  assert.equal(matchHint(okEdit, hints)?.id, "patch-reread")
  const clean = { status: "completed", tool: "edit", result: { content: [{ type: "text", text: "Edit applied" }] } }
  assert.equal(matchHint(clean, hints), undefined)
  const other = { status: "error", tool: "edit", error: { message: "permission denied" } }
  assert.equal(matchHint(other, hints), undefined)
  assert.equal(matchHint(undefined, hints), undefined)
})

test("tool filter: same text on another tool does not match", () => {
  const ev = { status: "error", tool: "read", error: { message: "Could not find oldString" } }
  assert.equal(matchHint(ev, hints), undefined)
  const upper = { status: "error", tool: "EDIT", error: { message: "Could not find oldString" } }
  assert.equal(matchHint(upper, hints)?.id, "patch-reread")
})

test("applyHint never throws", () => {
  assert.equal(applyHint(undefined, { text: "x" }), false)
  assert.equal(applyHint({ status: "error", error: null }, { text: "x" }), false)
  assert.equal(applyHint({ status: "completed", result: {} }, { text: "x" }), false)
  const frozen = Object.freeze({ status: "error", error: Object.freeze({ message: "m" }) })
  assert.equal(applyHint(frozen, { text: "x" }), false)
})

test("personal hints: same id replaces default, new ids are added after", () => {
  const dir = mkdtempSync(join(tmpdir(), "si-hints-"))
  try {
    const personal = join(dir, "hints.json")
    writeFileSync(
      personal,
      JSON.stringify({
        version: 1,
        hints: [
          { id: "patch-reread", tools: ["edit"], match: "stale", text: "Hint: mine" },
          { id: "mine", tools: ["bash"], match: "boom", text: "Hint: boom" },
        ],
      }),
    )
    const merged = loadHints(defaultPath, personal)
    assert.deepEqual(merged.map((h) => h.id).sort(), ["grep-regex", "memory-symlink-pathspec", "mine", "patch-add-file", "patch-reread", "read-offset-number", "zsh-status-var"])
    assert.equal(merged.filter((h) => h.id === "patch-reread").length, 1)
    const ev = { status: "error", tool: "edit", error: { message: "Could not find oldString" } }
    assert.equal(matchHint(ev, merged), undefined) // default regex replaced
    const ev2 = { status: "error", tool: "edit", error: { message: "file is stale" } }
    assert.equal(matchHint(ev2, merged).text, "Hint: mine")
    // Missing/garbage personal file is ignored.
    assert.equal(loadHints(defaultPath, join(dir, "nope.json")).length, loadHints(defaultPath).length)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test("validateHints reports errors", () => {
  assert.deepEqual(validateHints({ version: 1, hints: [] }), [])
  assert.ok(validateHints(null)[0].includes("JSON object"))
  assert.ok(validateHints({ version: 2, hints: [] }).some((e) => e.includes("version")))
  assert.ok(validateHints({ version: 1 }).some((e) => e.includes("'hints' must be an array")))
  const errs = validateHints({
    version: 1,
    hints: [
      { id: "a", tools: ["x"], match: "(", text: "t" },
      { id: "a", tools: [], match: "ok", text: "" },
      { tools: ["x"], match: "ok", text: "t" },
      "nope",
    ],
  })
  assert.ok(errs.some((e) => e.includes("invalid regex")))
  assert.ok(errs.some((e) => e.includes("duplicate id")))
  assert.ok(errs.some((e) => e.includes("tools")))
  assert.ok(errs.some((e) => e.includes("text")))
  assert.ok(errs.some((e) => e.includes("hints[2].id")))
  assert.ok(errs.some((e) => e.includes("hints[3]")))
})

test("read offset given as a range gets read-offset-number (both error formats)", () => {
  const hints = loadHints(defaultPath)
  const zod = { tool: "read", status: "error", error: { message: 'Invalid arguments for tool "read": - offset: Expected number Arguments provided: { "path": "/x", "offset": [280, 330] }' } }
  const schema = { tool: "read", status: "error", error: { message: 'The read tool was called with invalid arguments: SchemaError(Expected number | undefined, got [450,750] at ["offset"]). Please rewrite the input so it satisfies the expected schema.' } }
  assert.equal(matchHint(zod, hints)?.id, "read-offset-number")
  assert.equal(matchHint(schema, hints)?.id, "read-offset-number")
  const negative = { tool: "read", status: "error", error: { message: 'The read tool was called with invalid arguments: SchemaError(Expected a value greater than or equal to 0, got -40 at ["offset"]).' } }
  assert.equal(matchHint(negative, hints), undefined)
  const otherTool = { tool: "grep", status: "error", error: { message: "offset: Expected number" } }
  assert.equal(matchHint(otherTool, hints), undefined)
})

const shellEv = (text, exit = 128, tool = "shell") => ({ status: "completed", tool, result: { content: [{ type: "text", text }], metadata: { metadata: { exit } } } })

test("git pathspec beyond a symlink under .memory gets memory-symlink-pathspec", () => {
  for (const tool of ["shell", "bash"]) {
    assert.equal(matchHint(shellEv("fatal: pathspec '.memory/project/INDEX.md' is beyond a symbolic link", 128, tool), hints)?.id, "memory-symlink-pathspec")
  }
  assert.equal(matchHint(shellEv("fatal: pathspec '.memory' is beyond a symbolic link", 128), hints)?.id, "memory-symlink-pathspec")
  assert.equal(matchHint(shellEv("fatal: pathspec 'app/.memory/x.md' is beyond a symbolic link", 128), hints)?.id, "memory-symlink-pathspec")
})

test("memory-symlink-pathspec: other symlinks, exit 0 and other tools do not match", () => {
  assert.equal(matchHint(shellEv("fatal: pathspec 'config/link/file.txt' is beyond a symbolic link"), hints), undefined)
  assert.equal(matchHint(shellEv("fatal: pathspec '.memoryx/a' is beyond a symbolic link"), hints), undefined)
  assert.equal(matchHint(shellEv("fatal: pathspec '.memory/a' did not match any files"), hints), undefined)
  assert.equal(matchHint(shellEv("fatal: pathspec '.memory/a' is beyond a symbolic link", 0), hints), undefined)
  assert.equal(matchHint({ status: "error", tool: "read", error: { message: "pathspec '.memory/a' is beyond a symbolic link" } }, hints), undefined)
})

const personalHints = () => {
  const dir = mkdtempSync(join(tmpdir(), "si-hints-"))
  const personal = join(dir, "hints.json")
  writeFileSync(
    personal,
    JSON.stringify({
      version: 1,
      hints: [
        {
          id: "youtrack-work-type",
          tools: ["youtrack-mcp_get_allowed_values", "youtrack_get_allowed_values"],
          match: "Field not found:\\s*Work type\\b",
          text: "Hint: Work type is a draft-only classification in this user's task-drafting workflow, not a YouTrack custom field. Do not query or write it as a field. Resolve real fields from the project's live schema; do not assume a fixed list.",
        },
      ],
    }),
  )
  return { dir, merged: loadHints(defaultPath, personal), json: JSON.parse(readFileSync(personal, "utf8")) }
}

test("personal youtrack-work-type: matches Work type field-not-found only", () => {
  const { dir, merged, json } = personalHints()
  try {
    assert.deepEqual(validateHints(json), [])
    for (const tool of ["youtrack-mcp_get_allowed_values", "youtrack_get_allowed_values"]) {
      const ev = { status: "error", tool, error: { message: "FIELD_NOT_FOUND: Field not found: Work type" } }
      assert.equal(matchHint(ev, merged)?.id, "youtrack-work-type")
    }
    const lower = { status: "error", tool: "youtrack_get_allowed_values", error: { message: "field not found: work type" } }
    assert.equal(matchHint(lower, merged)?.id, "youtrack-work-type")
    const risk = { status: "error", tool: "youtrack-mcp_get_allowed_values", error: { message: "FIELD_NOT_FOUND: Field not found: Risk" } }
    assert.equal(matchHint(risk, merged), undefined)
    const wrongTool = { status: "error", tool: "youtrack-mcp_get_issue", error: { message: "FIELD_NOT_FOUND: Field not found: Work type" } }
    assert.equal(matchHint(wrongTool, merged), undefined)
    const success = { status: "completed", tool: "youtrack-mcp_get_allowed_values", result: { content: [{ type: "text", text: "Work type values" }] } }
    assert.equal(matchHint(success, merged), undefined)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
