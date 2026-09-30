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

test("buildInjection leaves inbox triage to dream", () => {
  const text = buildInjection("", 1)
  assert.ok(text.includes("Inbox: 1 unprocessed event. The nightly `learn dream` triages them; do not triage the inbox in-session unless the user asks."))
  assert.ok(!text.includes("Triage with the self-improving skill"))
  assert.ok(!text.includes("Proposed, waiting"))
  assert.ok(!text.includes("At a natural pause"))
})

test("buildInjection: exact layout with candidates, proposed and inbox", () => {
  const idx = "# idx\n- [a.b](candidates/a.b.md) — correction ×2: do x\n"
  assert.equal(
    buildInjection(idx, 3, ["a.b", "c.d"]),
    [
      "<self-improving>",
      "Behavior learnings for this project live in ./.learnings (managed by the `learn` CLI).",
      "Candidates (1):",
      "- [a.b](candidates/a.b.md) — correction ×2: do x",
      "Proposed, waiting for the user's decision (2): a.b, c.d",
      "Inbox: 3 unprocessed events. The nightly `learn dream` triages them; do not triage the inbox in-session unless the user asks.",
      "At a natural pause (end of a task), offer the proposed candidates to the user with the self-improving skill. Never mid-task.",
      "</self-improving>",
    ].join("\n"),
  )
})

test("buildInjection: proposed and offer lines only when P > 0", () => {
  const idx = "- [a.b](candidates/a.b.md) — correction ×2: do x\n"
  for (const keys of [undefined, []]) {
    const none = buildInjection(idx, 0, keys)
    assert.ok(!none.includes("Proposed, waiting"))
    assert.ok(!none.includes("At a natural pause"))
    assert.ok(!none.includes("Inbox:"))
  }
  const some = buildInjection(idx, 0, ["a.b"])
  assert.ok(some.includes("Proposed, waiting for the user's decision (1): a.b\n"))
  assert.ok(some.includes("At a natural pause (end of a task), offer the proposed candidates"))
  assert.ok(!some.includes("Inbox:"))
  assert.equal(buildInjection("", 0, ["a.b"]), undefined)
})

test("buildInjection: proposed list is capped at 10 keys with an ellipsis", () => {
  const keys = Array.from({ length: 12 }, (_, i) => `k${i + 1}`)
  const text = buildInjection("", 1, keys)
  assert.ok(text.includes("(12): k1, k2, k3, k4, k5, k6, k7, k8, k9, k10, …\n"), text)
  assert.ok(!text.includes("k11"))
  const ten = buildInjection("", 1, keys.slice(0, 10))
  assert.ok(ten.includes("(10): k1, k2, k3, k4, k5, k6, k7, k8, k9, k10\n"), ten)
  assert.ok(!ten.includes("…"))
})

test("buildInjection: inbox line uses singular and plural", () => {
  assert.ok(buildInjection("", 1).includes("Inbox: 1 unprocessed event. "))
  assert.ok(buildInjection("", 2).includes("Inbox: 2 unprocessed events. "))
  assert.ok(!buildInjection("- [a](candidates/a.md) — x\n", 0).includes("Inbox:"))
})

test("buildInjection", () => {
  assert.equal(buildInjection("", 0), undefined)
  const one = buildInjection("# idx\n- [a.b](candidates/a.b.md) — correction ×2: do x\n", 3)
  assert.ok(one.startsWith("<self-improving>") && one.endsWith("</self-improving>"))
  assert.ok(one.includes("a.b") && one.includes("3 unprocessed"))
  assert.ok(buildInjection("", 1).includes("1 unprocessed event."))

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

// Real payload observed on OpenCode v2.0.20 (`shell` tool, status completed).
const V2_SHELL = (exit, text) => ({
  tool: "shell",
  status: "completed",
  result: {
    output: text,
    title: "shell",
    metadata: {
      metadata: { status: "completed", truncated: false, exit },
      content: [{ type: "text", text }, { type: "text", text: `Exited with code ${exit}` }],
    },
  },
})

test("detectToolFailure: real v2.0.20 shell payload — exit 3 captured, exit 0 never scanned", async () => {
  const { detectToolFailure } = await import("../skills/self-improving/opencode/learn-core.js")
  assert.deepEqual(detectToolFailure(V2_SHELL(3, "boom\n")), { tool: "shell", head: "exit 3: boom" })
  // Successful command whose output merely *contains* failure-looking text (e.g. grep over source).
  const noisy = 'sed -n: 189-  if (/Traceback/.test(hit)) {\nError: handling code\n"Exited with code 3" in a string\n'
  assert.equal(detectToolFailure(V2_SHELL(0, noisy)), undefined)
})
