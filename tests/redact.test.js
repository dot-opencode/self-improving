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
