import assert from "node:assert/strict"
import { test } from "node:test"

import { buildInjection, detectToolFailure, inboxLine, redact } from "../skills/self-improving/opencode/learn-core.js"

test("redacts each secret pattern", () => {
  const cases = [
    ["Authorization: Bearer abc.def-123", "abc.def-123"],
    ["key sk-abcdEFGH12345678 end", "sk-abcdEFGH12345678"],
    ["token ghp_AbCdEf123456 end", "ghp_AbCdEf123456"],
    ["aws AKIAABCDEFGHIJKLMNOP end", "AKIAABCDEFGHIJKLMNOP"],
    ["slack xoxb-123-456-abc end", "xoxb-123-456-abc"],
    ["clone https://user:hunter2@example.com/repo.git", "hunter2"],
    ["API_KEY=supersecret", "supersecret"],
    ["api-key: supersecret", "supersecret"],
    ["password = hunter2", "hunter2"],
    ["Secret: abcdef", "abcdef"],
  ]
  for (const [input, secret] of cases) {
    const out = redact(input)
    assert.ok(!out.includes(secret), `${input} -> ${out}`)
    assert.ok(out.includes("[redacted]"), out)
  }
})

test("redacts extended secret patterns", () => {
  const cases = [
    ['{"password":"hunter2"}', "hunter2"],
    ['{"api_key": "abc123xyz", "x": 1}', "abc123xyz"],
    ["passwd=hunter2", "hunter2"],
    ["Authorization: Basic dXNlcjpwYXNzd29yZA==", "dXNlcjpwYXNzd29yZA=="],
    ["pat github_pat_11ABCDEF_xyz123 end", "github_pat_11ABCDEF_xyz123"],
    ["pat glpat-abcDEF123456 end", "glpat-abcDEF123456"],
    ["tok gho_abcDEF123456 end", "gho_abcDEF123456"],
    ["tok ghs_abcDEF123456 end", "ghs_abcDEF123456"],
    ["jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abc-DEF_123 end", "eyJhbGciOiJIUzI1NiJ9"],
  ]
  for (const [input, secret] of cases) {
    const out = redact(input)
    assert.ok(!out.includes(secret), `${input} -> ${out}`)
    assert.ok(out.includes("[redacted]"), out)
  }
})

test("redacts home directory prefixes", () => {
  assert.equal(redact("ENOENT open /Users/kurbezz/x"), "ENOENT open ~/x")
  assert.equal(redact("cd /home/alice/proj/src"), "cd ~/proj/src")
  assert.ok(!redact("failed at /Users/kurbezz/work/a.js:1").includes("kurbezz"))
})

test("leaves ordinary text alone", () => {
  assert.equal(redact("pytest: ModuleNotFoundError conftest"), "pytest: ModuleNotFoundError conftest")
})

test("truncates at 300 by default and honors max", () => {
  assert.equal(redact("a".repeat(1000)).length, 300)
  assert.equal(redact("a".repeat(1000), 50).length, 50)
  assert.equal(redact("a".repeat(300)).length, 300)
})

test("detectToolFailure: error status", () => {
  assert.deepEqual(detectToolFailure({ status: "error", tool: "bash", error: { message: "boom" } }), { tool: "bash", head: "boom" })
})

test("detectToolFailure: completed with failure markers", () => {
  assert.equal(detectToolFailure({ status: "completed", tool: "bash", result: { output: "exit code: 2\npytest failed" } })?.tool, "bash")
  assert.ok(detectToolFailure({ status: "completed", tool: "edit", result: { content: [{ type: "text", text: "oldString not found in content" }] } }))
  assert.ok(detectToolFailure({ status: "completed", tool: "bash", result: "npm ERR! code E404" }))
  assert.ok(detectToolFailure({ status: "completed", tool: "bash", result: { output: "Traceback (most recent call last)" } }))
  assert.equal(detectToolFailure({ status: "completed", tool: "bash", result: { output: "exit code: 0\nall good" } }), undefined)
  assert.equal(detectToolFailure({ status: "completed", tool: "read", result: "hello" }), undefined)
  assert.equal(detectToolFailure(undefined), undefined)
})

test("detectToolFailure: numeric exit (OpenCode V2 bash)", () => {
  const r = detectToolFailure({ status: "completed", tool: "bash", result: { output: { exit: 2, output: "boom" } } })
  assert.deepEqual(r, { tool: "bash", head: "exit 2: boom" })
  assert.equal(detectToolFailure({ status: "completed", tool: "bash", result: { exit: 3, output: "a\nlast line\n\n" } })?.head, "exit 3: last line")
  assert.equal(detectToolFailure({ status: "completed", tool: "bash", result: { metadata: { exit: 1 }, output: "nope" } })?.head, "exit 1: nope")
  assert.ok(detectToolFailure({ status: "completed", tool: "bash", result: "Command exited with code 2." }))
  assert.ok(detectToolFailure({ status: "completed", tool: "shell", result: "Command exited with code 127." }))
  assert.equal(detectToolFailure({ status: "completed", tool: "bash", result: { exit: 0, output: "fine" } }), undefined)
  assert.equal(detectToolFailure({ status: "completed", tool: "bash", result: { output: { exit: 0, output: "fine" } } }), undefined)
  assert.equal(detectToolFailure({ status: "completed", tool: "bash", result: "Command exited with code 0." }), undefined)
})

test("detectToolFailure: completed heuristics only for allowlisted tools", () => {
  assert.equal(detectToolFailure({ status: "completed", tool: "read", result: "Error: foo" }), undefined)
  assert.equal(detectToolFailure({ status: "completed", tool: "read", result: { output: "Error: foo" } }), undefined)
  assert.equal(detectToolFailure({ status: "completed", tool: "grep", result: { output: "Traceback (most recent call last)" } }), undefined)
  assert.equal(detectToolFailure({ status: "completed", tool: "grep", result: "exit code: 2" }), undefined)
  // error status stays tool-agnostic
  assert.deepEqual(detectToolFailure({ status: "error", tool: "read", error: "nope" }), { tool: "read", head: "nope" })
  // edit-like tools
  for (const tool of ["edit", "patch", "write", "multiedit", "apply_patch"]) {
    assert.ok(detectToolFailure({ status: "completed", tool, result: "oldString not found in content" }), tool)
  }
  assert.equal(detectToolFailure({ status: "completed", tool: "edit", result: "Error: foo" }), undefined)
})

test("detectToolFailure: traceback head includes last line", () => {
  const out = "Traceback (most recent call last):\n  File \"a.py\", line 1\nValueError: bad"
  assert.equal(detectToolFailure({ status: "completed", tool: "bash", result: out })?.head, "Traceback (most recent call last):: ValueError: bad")
})

test("buildInjection tells the agent when to triage", () => {
  const text = buildInjection("", 1)
  assert.ok(text.includes("Triage with the self-improving skill at a natural pause (end of task or right after a user correction), not mid-task."))
  assert.ok(!text.includes("unprompted"))
})

test("buildInjection", () => {
  assert.equal(buildInjection("", 0), undefined)
  const one = buildInjection("# idx\n- [a.b](candidates/a.b.md) — correction ×2: do x\n", 3)
  assert.ok(one.startsWith("<self-improving>") && one.endsWith("</self-improving>"))
  assert.ok(one.includes("a.b") && one.includes("3 unprocessed"))
  assert.ok(buildInjection("", 1).includes("1 unprocessed event "))

  const many = Array.from({ length: 500 }, (_, i) => `- [test.k${i}](candidates/test.k${i}.md) — correction ×2: ${"d".repeat(60)}`).join("\n")
  const big = buildInjection(many, 5)
  assert.ok(big.length <= 4096, String(big.length))
  assert.ok(big.includes("truncated"))
  assert.ok(big.endsWith("</self-improving>"))
})

test("inboxLine is a single JSON line", () => {
  const line = inboxLine({ a: "x\ny" })
  assert.ok(line.endsWith("\n"))
  assert.equal(line.trim().split("\n").length, 1)
  assert.deepEqual(JSON.parse(line), { a: "x\ny" })
})
