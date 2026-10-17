import assert from "node:assert/strict"
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, utimesSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"

import plugin from "../skills/self-improving/opencode/self-improving.js"

const tick = () => new Promise((resolve) => setTimeout(resolve, 20))

function fixture({ learnings = true, parentID } = {}) {
  const root = mkdtempSync(join(tmpdir(), "si-plugin-"))
  const store = join(root, "store")
  const project = join(root, "myproj")
  mkdirSync(join(store, "work", "inbox"), { recursive: true })
  mkdirSync(project, { recursive: true })
  if (learnings) symlinkSync(join(store, "work"), join(project, ".learnings"))
  return { root, store, project, parentID }
}

function fakeCtx(fx) {
  const hooks = {}
  const queue = []
  const getCalls = []
  let wake
  const ctx = {
    tool: { hook: async (name, fn) => void (hooks[`tool:${name}`] = fn) },
    session: {
      hook: async (name, fn) => void (hooks[`session:${name}`] = fn),
      get: async ({ sessionID }) => {
        getCalls.push(sessionID)
        return {
          parentID: sessionID === "ses_sub" ? "ses_parent" : undefined,
          location: { directory: fx.dirs?.[sessionID] ?? fx.project },
        }
      },
    },
    event: {
      subscribe({ signal }) {
        return (async function* () {
          while (!signal.aborted) {
            if (queue.length) {
              yield queue.shift()
              continue
            }
            await new Promise((resolve) => {
              wake = resolve
              signal.addEventListener("abort", resolve, { once: true })
            })
          }
        })()
      },
    },
  }
  return {
    ctx,
    hooks,
    getCalls,
    emit(event) {
      queue.push(event)
      wake?.()
    },
  }
}

function inboxEvents(fx) {
  const dir = join(fx.store, "work", "inbox")
  return readdirSync(dir)
    .filter((name) => name.endsWith(".jsonl"))
    .flatMap((name) => readFileSync(join(dir, name), "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line)))
}

function fakeSpawn(calls) {
  return (command, args, options) => {
    calls.push({ command, args, cwd: options.cwd })
    const handlers = {}
    setTimeout(() => handlers.close?.(0), 30)
    return { on: (event, fn) => void (handlers[event] = fn) }
  }
}

async function start(fx, extra = {}) {
  const fake = fakeCtx(fx)
  const calls = []
  const cleanup = await plugin.setup(fake.ctx, { spawn: fakeSpawn(calls), storeRoot: fx.store, ...extra })
  return { ...fake, calls, cleanup }
}

const prompt = (sessionID, messageID, text) => ({ sessionID, messageID, prompt: { text } })

test("plugin shape", () => {
  assert.equal(plugin.id, "self-improving")
  assert.equal(typeof plugin.setup, "function")
})

test("writes failure and correction lines; skips first message", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    // First message is never captured, even if it matches.
    await p.hooks["session:prompt"](prompt("ses_1", "m1", "Нет, я про другое"))
    assert.equal(inboxEvents(fx).length, 0)

    await p.hooks["tool:execute.after"]({
      status: "error",
      tool: "bash",
      sessionID: "ses_1",
      messageID: "m2",
      error: { message: "exit 2: pytest: ModuleNotFoundError Bearer abc123token" },
    })
    // Duplicate head in the same session is deduped.
    await p.hooks["tool:execute.after"]({
      status: "error",
      tool: "bash",
      sessionID: "ses_1",
      error: { message: "exit 2: pytest: ModuleNotFoundError Bearer abc123token" },
    })

    await p.hooks["session:prompt"](prompt("ses_1", "m3", "Нет, я про другое"))
    // Same messageID is deduped.
    await p.hooks["session:prompt"](prompt("ses_1", "m3", "Нет, я про другое"))
    // Weak marker after tool action is captured with prev_tool.
    await p.hooks["tool:execute.after"]({ status: "completed", tool: "edit", sessionID: "ses_1", result: "ok" })
    await p.hooks["session:prompt"](prompt("ses_1", "m4", "Почему ci упал?"))
    // Weak marker without a tool action in between is not.
    await p.hooks["session:prompt"](prompt("ses_1", "m5", "почему так"))

    const events = inboxEvents(fx)
    assert.equal(events.length, 3, JSON.stringify(events))
    const [failure, correction, weak] = events
    assert.equal(failure.kind, "failure")
    assert.equal(failure.tool, "bash")
    assert.equal(failure.project, "myproj")
    assert.equal(failure.session, "ses_1")
    assert.ok(!failure.head.includes("abc123token"))
    assert.match(failure.ts, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/)
    assert.equal(correction.kind, "correction")
    assert.equal(correction.tier, "strong")
    assert.equal(correction.snippet, "Нет, я про другое")
    assert.equal(correction.marker, "^нет[,.\\-—]")
    assert.equal(weak.tier, "weak")
    assert.equal(weak.prev_tool, "edit")
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("completed-with-failure result is captured; subagent sessions capture no prompts", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    await p.hooks["tool:execute.after"]({ status: "completed", tool: "bash", sessionID: "ses_2", result: { output: "exit code: 1\nboom" } })
    await p.hooks["session:prompt"](prompt("ses_sub", "s1", "hello"))
    await p.hooks["session:prompt"](prompt("ses_sub", "s2", "Нет, я про другое"))
    const events = inboxEvents(fx)
    assert.equal(events.length, 1)
    assert.equal(events[0].kind, "failure")
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("context injection is pushed once and within 4096 chars", async () => {
  const fx = fixture()
  const lines = Array.from({ length: 300 }, (_, i) => `- [test.k${i}](candidates/test.k${i}.md) — correction ×2: ${"x".repeat(50)}`)
  writeFileSync(join(fx.store, "work", "INDEX.md"), `# Index\n${lines.join("\n")}\n`)
  writeFileSync(join(fx.store, "work", "inbox", "2026-01-01.jsonl"), `{"ts":"2026-01-01T00:00:00Z","kind":"failure"}\n{"ts":"2026-01-02T00:00:00Z","kind":"failure"}\n`)
  writeFileSync(join(fx.store, "work", "inbox", ".ack"), "2026-01-01T12:00:00Z\n")
  const p = await start(fx)
  try {
    const ev = { sessionID: "ses_1", system: [{ type: "text", text: "base" }] }
    await p.hooks["session:context"](ev)
    assert.equal(ev.system.length, 2)
    const text = ev.system[1].text
    assert.ok(text.startsWith("<self-improving>") && text.endsWith("</self-improving>"))
    assert.ok(text.length <= 4096, String(text.length))
    assert.ok(text.includes("1 unprocessed event."), text.slice(-300))
    await p.hooks["session:context"](ev)
    assert.equal(ev.system.length, 2)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("no injection when there is nothing to report", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    const ev = { sessionID: "ses_1", system: [] }
    await p.hooks["session:context"](ev)
    assert.equal(ev.system.length, 0)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("backup spawn is deduped per directory while running", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    p.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } })
    p.emit({ type: "session.execution.failed", data: { sessionID: "ses_2" } })
    p.emit({ type: "session.execution.interrupted", data: { sessionID: "ses_3" } })
    p.emit({ type: "session.something.else", data: { sessionID: "ses_4" } })
    await tick()
    assert.equal(p.calls.length, 1)
    assert.equal(p.calls[0].cwd, fx.project)
    assert.equal(p.calls[0].command, "bash")
    assert.equal(p.calls[0].args[1], "backup")
    assert.match(p.calls[0].args[0], /bin\/learn$/)

    await new Promise((resolve) => setTimeout(resolve, 80))
    p.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } })
    await tick()
    assert.equal(p.calls.length, 2)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("inert without .learnings symlink", async () => {
  const fx = fixture({ learnings: false })
  writeFileSync(join(fx.store, "work", "INDEX.md"), "- [a.b](candidates/a.b.md) — correction ×2: x\n")
  const p = await start(fx)
  try {
    await p.hooks["session:prompt"](prompt("ses_1", "m1", "hello"))
    await p.hooks["tool:execute.after"]({ status: "error", tool: "bash", sessionID: "ses_1", error: { message: "boom" } })
    await p.hooks["session:prompt"](prompt("ses_1", "m2", "Нет, я про другое"))
    const ev = { sessionID: "ses_1", system: [] }
    await p.hooks["session:context"](ev)
    p.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } })
    await tick()
    assert.equal(inboxEvents(fx).length, 0)
    assert.equal(ev.system.length, 0)
    assert.equal(p.calls.length, 0)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("hooks never throw", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    await p.hooks["tool:execute.after"](undefined)
    await p.hooks["session:prompt"]({})
    await p.hooks["session:context"]({ sessionID: "ses_1" })
    await p.hooks["session:prompt"](prompt("nope", "m1", "x"))
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("cleanup aborts subscription", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    p.cleanup()
    p.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } })
    await tick()
    assert.equal(p.calls.length, 0)
  } finally {
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("confirmations are never written by the plugin", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    await p.hooks["session:prompt"](prompt("ses_1", "m1", "hello"))
    await p.hooks["tool:execute.after"]({ status: "completed", tool: "edit", sessionID: "ses_1", result: "ok" })
    await p.hooks["session:prompt"](prompt("ses_1", "m2", "ок"))
    await p.hooks["session:prompt"](prompt("ses_1", "m3", "LGTM"))
    assert.equal(inboxEvents(fx).length, 0)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("structured bash exit failure is captured; read tool output is ignored", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    await p.hooks["tool:execute.after"]({ status: "completed", tool: "bash", sessionID: "ses_1", result: { output: { exit: 2, output: "boom" } } })
    await p.hooks["tool:execute.after"]({ status: "completed", tool: "read", sessionID: "ses_1", result: "Error: foo" })
    const events = inboxEvents(fx)
    assert.equal(events.length, 1)
    assert.equal(events[0].head, "exit 2: boom")
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("failure heads have home paths redacted", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    await p.hooks["tool:execute.after"]({ status: "error", tool: "bash", sessionID: "ses_1", error: { message: "ENOENT open /Users/kurbezz/x" } })
    assert.equal(inboxEvents(fx)[0].head, "ENOENT open ~/x")
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("inert when .learnings escapes the store or has a bad profile name", async () => {
  for (const kind of ["parent", "git", "outside"]) {
    const fx = fixture({ learnings: false })
    let target
    if (kind === "parent") target = ".."
    else if (kind === "git") {
      target = join(fx.store, ".git")
      mkdirSync(target, { recursive: true })
    } else {
      target = join(fx.root, "outside", "work")
      mkdirSync(target, { recursive: true })
    }
    symlinkSync(target, join(fx.project, ".learnings"))
    writeFileSync(join(fx.store, "work", "INDEX.md"), "- [a.b](candidates/a.b.md) — correction ×2: x\n")
    const p = await start(fx)
    try {
      await p.hooks["session:prompt"](prompt("ses_1", "m1", "hello"))
      await p.hooks["tool:execute.after"]({ status: "error", tool: "bash", sessionID: "ses_1", error: { message: "boom" } })
      await p.hooks["session:prompt"](prompt("ses_1", "m2", "Нет, я про другое"))
      const ev = { sessionID: "ses_1", system: [] }
      await p.hooks["session:context"](ev)
      p.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } })
      await tick()
      assert.equal(inboxEvents(fx).length, 0, kind)
      assert.equal(ev.system.length, 0, kind)
      assert.equal(p.calls.length, 0, kind)
      if (kind === "git") assert.deepEqual(readdirSync(join(fx.store, ".git")), [])
      if (kind === "outside") assert.deepEqual(readdirSync(join(fx.root, "outside", "work")), [])
    } finally {
      p.cleanup()
      rmSync(fx.root, { recursive: true, force: true })
    }
  }
})

test("backup is deduped per store across different project dirs", async () => {
  const fx = fixture()
  const project2 = join(fx.root, "other")
  mkdirSync(project2, { recursive: true })
  symlinkSync(join(fx.store, "work"), join(project2, ".learnings"))
  fx.dirs = { ses_b: project2 }
  const p = await start(fx)
  try {
    p.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_a" } })
    p.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_b" } })
    await tick()
    assert.equal(p.calls.length, 1)
    // Which session wins the race is not deterministic; only one backup must run.
    assert.ok([fx.project, project2].includes(p.calls[0].cwd))
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("no context injection for subagent sessions", async () => {
  const fx = fixture()
  writeFileSync(join(fx.store, "work", "INDEX.md"), "- [a.b](candidates/a.b.md) — correction ×2: x\n")
  const p = await start(fx)
  try {
    const sub = { sessionID: "ses_sub", system: [] }
    await p.hooks["session:context"](sub)
    assert.equal(sub.system.length, 0)
    const main = { sessionID: "ses_1", system: [] }
    await p.hooks["session:context"](main)
    assert.equal(main.system.length, 1)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("sessions map is capped at 200 (oldest evicted) and session.deleted drops state", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    for (let i = 0; i < 201; i++) await p.hooks["session:prompt"](prompt(`ses_${i}`, `m${i}`, "hello"))
    assert.equal(p.getCalls.length, 201)
    await p.hooks["session:prompt"](prompt("ses_200", "x1", "hello"))
    assert.equal(p.getCalls.length, 201, "newest session is still cached")
    await p.hooks["session:prompt"](prompt("ses_0", "x2", "hello"))
    assert.equal(p.getCalls.length, 202, "oldest session was evicted")

    p.emit({ type: "session.deleted", data: { sessionID: "ses_200" } })
    p.emit({ type: "session.deleted", data: { info: { id: "ses_199" } } })
    await tick()
    await p.hooks["session:prompt"](prompt("ses_200", "x3", "hello"))
    await p.hooks["session:prompt"](prompt("ses_199", "x4", "hello"))
    assert.equal(p.getCalls.length, 204)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("per-session dedupe set is cleared after 500 entries", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    const fail = (n) => p.hooks["tool:execute.after"]({ status: "error", tool: "bash", sessionID: "ses_1", error: { message: `fail ${n}` } })
    await fail(0)
    await fail(0)
    assert.equal(inboxEvents(fx).length, 1)
    for (let i = 1; i <= 500; i++) await fail(i)
    assert.equal(inboxEvents(fx).length, 501)
    await fail(0) // set was cleared, so the first head is written again
    assert.equal(inboxEvents(fx).length, 502)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("unacked count skips day files older than the ack date", async () => {
  const fx = fixture()
  writeFileSync(join(fx.store, "work", "inbox", "2026-01-01.jsonl"), `{"kind":"failure"}\n{"kind":"failure"}\n`) // no ts => would count if read
  writeFileSync(join(fx.store, "work", "inbox", "2026-01-03.jsonl"), `{"ts":"2026-01-03T00:00:00Z","kind":"failure"}\n`)
  writeFileSync(join(fx.store, "work", "inbox", ".ack"), "2026-01-02T12:00:00Z\n")
  const p = await start(fx)
  try {
    const ev = { sessionID: "ses_1", system: [] }
    await p.hooks["session:context"](ev)
    assert.ok(ev.system[0].text.includes("Inbox: 1 unprocessed event. The nightly `learn dream` triages them"), ev.system[0]?.text)
    assert.ok(!ev.system[0].text.includes("Triage with the self-improving skill"))
    assert.ok(!ev.system[0].text.includes("Proposed, waiting"))
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("context injection lists proposed candidate keys from the store", async () => {
  const fx = fixture()
  const cands = join(fx.store, "work", "candidates")
  mkdirSync(cands, { recursive: true })
  const cand = (status, extra = "") =>
    `---\ndescription: d\npattern-key: k\nkind: correction\nseen: 3\nstatus: ${status}\n---\n## Future behavior\nx\n${extra}`
  writeFileSync(join(cands, "test.zeta.md"), cand("proposed"))
  writeFileSync(join(cands, "Test.upper.md"), cand("proposed"))
  writeFileSync(join(cands, "test.alpha.md"), cand("proposed"))
  writeFileSync(join(cands, "test.plain.md"), cand("candidate"))
  // `status: proposed` outside the first frontmatter block does not count.
  writeFileSync(join(cands, "test.body.md"), cand("candidate", "\nstatus: proposed\n"))
  writeFileSync(join(cands, "notes.txt"), cand("proposed"))
  writeFileSync(join(fx.store, "work", "INDEX.md"), "# Index\n- [test.plain](candidates/test.plain.md) — correction ×1: d\n")
  const p = await start(fx)
  try {
    const ev = { sessionID: "ses_1", system: [] }
    await p.hooks["session:context"](ev)
    const text = ev.system[0].text
    assert.ok(text.includes("Proposed, waiting for the user's decision (3): Test.upper, test.alpha, test.zeta\n"), text)
    assert.ok(text.includes("At a natural pause (end of a task), offer the proposed candidates"))
    assert.ok(!text.includes("Inbox:"))
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("context injection without a candidates directory has no proposed line", async () => {
  const fx = fixture()
  writeFileSync(join(fx.store, "work", "INDEX.md"), "# Index\n- [a.b](candidates/a.b.md) — correction ×1: d\n")
  const p = await start(fx)
  try {
    const ev = { sessionID: "ses_1", system: [] }
    await p.hooks["session:context"](ev)
    assert.ok(ev.system[0].text.includes("Candidates (1):"))
    assert.ok(!ev.system[0].text.includes("Proposed, waiting"))
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("hook registration failure is logged unconditionally", async () => {
  const fx = fixture()
  const fake = fakeCtx(fx)
  const logged = []
  fake.ctx.log = { error: (entry) => void logged.push(entry) }
  fake.ctx.tool.hook = async () => {
    throw new Error("nope")
  }
  const cleanup = await plugin.setup(fake.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store })
  try {
    assert.ok(logged.some((entry) => entry.message === "hook registration failed"), JSON.stringify(logged))
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("hint is applied synchronously, before the returned promise settles", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    const ev = { status: "error", tool: "edit", sessionID: "ses_1", error: { message: "Could not find oldString in x.js" } }
    const pending = p.hooks["tool:execute.after"](ev)
    // No await yet: the mutation must already be visible.
    assert.ok(ev.error.message.includes("Hint: your copy of this file is stale"), ev.error.message)
    await pending
    assert.equal(ev.error.message.split("Hint: your copy").length, 2)

    const shell = {
      status: "completed",
      tool: "shell",
      sessionID: "ses_1",
      result: { content: [{ type: "text", text: "zsh: read-only variable: status" }], metadata: { metadata: { exit: 1 } } },
    }
    const pending2 = p.hooks["tool:execute.after"](shell)
    assert.equal(shell.result.content.length, 2)
    await pending2
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("inbox failure event carries the hint id; the hint text is not in the head", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    await p.hooks["tool:execute.after"]({ status: "error", tool: "edit", sessionID: "ses_1", error: { message: "Could not find oldString in x.js" } })
    await p.hooks["tool:execute.after"]({ status: "error", tool: "bash", sessionID: "ses_1", error: { message: "exit 2: other" } })
    const [hinted, plain] = inboxEvents(fx)
    assert.equal(hinted.hint, "patch-reread")
    assert.ok(!hinted.head.includes("Hint:"), hinted.head)
    assert.equal(plain.hint, undefined)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("inert session still gets the hint but writes nothing", async () => {
  const fx = fixture({ learnings: false })
  const p = await start(fx)
  try {
    const ev = { status: "error", tool: "grep", sessionID: "ses_1", error: { message: "regex parse error: look-around" } }
    const pending = p.hooks["tool:execute.after"](ev)
    assert.ok(ev.error.message.includes("Hint: grep uses ripgrep"))
    await pending
    assert.equal(inboxEvents(fx).length, 0)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("subagent session gets the hint too", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    const ev = { status: "error", tool: "patch", sessionID: "ses_sub", error: { message: "Invalid Add File line: x" } }
    await p.hooks["tool:execute.after"](ev)
    assert.ok(ev.error.message.includes("Add File body must start"))
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("personal hints.json in the store is merged and overrides by id", async () => {
  const fx = fixture()
  writeFileSync(
    join(fx.store, "hints.json"),
    JSON.stringify({ version: 1, hints: [{ id: "mine", tools: ["bash"], match: "boom", text: "Hint: mine" }] }),
  )
  const p = await start(fx)
  try {
    const ev = { status: "error", tool: "bash", sessionID: "ses_1", error: { message: "exit 1: boom" } }
    await p.hooks["tool:execute.after"](ev)
    assert.ok(ev.error.message.endsWith("Hint: mine"))
    assert.equal(inboxEvents(fx)[0].hint, "mine")
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("unacked count uses ats (arrival key) when present, mixed with plain events", async () => {
  const fx = fixture()
  // Backfill: old ts, arrival after the ack. Plain old event is acked. Plain new event counts.
  writeFileSync(
    join(fx.store, "work", "inbox", "2026-10-01-backfill.jsonl"),
    `{"ts":"2026-01-05T08:00:00Z","ats":"2026-10-01T09:00:00Z","kind":"failure"}\n{"ts":"2026-01-06T08:00:00Z","kind":"failure"}\n{"ts":"2026-10-01T10:00:00Z","kind":"failure"}\n`,
  )
  writeFileSync(join(fx.store, "work", "inbox", ".ack"), "2026-09-30T00:00:00Z\n")
  const p = await start(fx)
  try {
    const ev = { sessionID: "ses_1", system: [] }
    await p.hooks["session:context"](ev)
    assert.equal(ev.system.length, 1)
    assert.ok(ev.system[0].text.includes("2 unprocessed events"), ev.system[0].text)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

function withCommands(fx) {
  const fake = fakeCtx(fx)
  const commands = {}
  const synthetic = []
  const prompts = []
  fake.ctx.command = { transform: async (fn) => fn({ add: (c) => void (commands[c.name] = c) }) }
  fake.ctx.session.synthetic = async (input) => void synthetic.push(input)
  fake.ctx.session.prompt = async (input) => void prompts.push(input)
  return { ...fake, commands, synthetic, prompts }
}

function candidate(fx, profile, key, status) {
  const dir = join(fx.store, profile, "candidates")
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, `${key}.md`), `---\nkey: ${key}\nstatus: ${status}\n---\nbody\n`)
}

async function startCommands(fx) {
  const w = withCommands(fx)
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store })
  return { ...w, cleanup }
}

test("registers /learn-check and /learn-review", async () => {
  const fx = fixture()
  const p = await startCommands(fx)
  try {
    assert.deepEqual(Object.keys(p.commands).sort(), ["learn-check", "learn-review"])
    assert.ok(p.commands["learn-check"].description)
    assert.ok(p.commands["learn-review"].description)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("/learn-check reports proposals across profiles without .learnings", async () => {
  const fx = fixture({ learnings: false })
  candidate(fx, "work", "a.b", "proposed")
  candidate(fx, "work", "c.d", "proposed")
  candidate(fx, "work", "e.f", "candidate")
  candidate(fx, "home", "x.y", "candidate")
  const p = await startCommands(fx)
  try {
    await p.commands["learn-check"].execute({ sessionID: "ses_1", prompt: { text: "" } })
    assert.equal(p.synthetic.length, 1)
    const text = p.synthetic[0].text
    assert.match(text, /^self-improving: 2 proposals waiting for review\./)
    assert.match(text, /- work: proposed 2 \(a\.b, c\.d\); candidates 3; inbox 0 unprocessed/)
    assert.match(text, /- home: proposed 0; candidates 1; inbox 0 unprocessed/)
    assert.ok(text.indexOf("- home") < text.indexOf("- work"))
    assert.match(text, /Run \/learn-review/)
    assert.equal(p.prompts.length, 0)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("/learn-check with no proposals", async () => {
  const fx = fixture()
  candidate(fx, "work", "a.b", "candidate")
  const p = await startCommands(fx)
  try {
    await p.commands["learn-check"].execute({ sessionID: "ses_1", prompt: { text: "" } })
    const text = p.synthetic[0].text
    assert.match(text, /^self-improving: no proposals waiting for review\./)
    assert.match(text, /- work: proposed 0; candidates 1/)
    assert.doesNotMatch(text, /\/learn-review/)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("/learn-review prompts the model with keys, paths and --profile; arg filters", async () => {
  const fx = fixture({ learnings: false })
  candidate(fx, "work", "a.b", "proposed")
  candidate(fx, "home", "c.d", "proposed")
  const p = await startCommands(fx)
  try {
    await p.commands["learn-review"].execute({ sessionID: "ses_1", delivery: "immediate", prompt: { text: "  " } })
    assert.equal(p.prompts.length, 1)
    const call = p.prompts[0]
    assert.equal(call.sessionID, "ses_1")
    assert.equal(call.delivery, "immediate")
    assert.match(call.text, /self-improving/)
    assert.match(call.text, /work: a\.b — .*candidates[\\/]a\.b\.md/)
    assert.match(call.text, /home: c\.d — /)
    assert.match(call.text, /learn --profile/)

    await p.commands["learn-review"].execute({ sessionID: "ses_1", prompt: { text: "home" } })
    assert.doesNotMatch(p.prompts[1].text, /work: a\.b/)
    assert.match(p.prompts[1].text, /home: c\.d/)

    await p.commands["learn-review"].execute({ sessionID: "ses_1", prompt: { text: "a.b" } })
    assert.match(p.prompts[2].text, /work: a\.b/)
    assert.doesNotMatch(p.prompts[2].text, /home: c\.d/)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("/learn-review with nothing proposed uses synthetic only", async () => {
  const fx = fixture()
  candidate(fx, "work", "a.b", "candidate")
  const p = await startCommands(fx)
  try {
    await p.commands["learn-review"].execute({ sessionID: "ses_1", prompt: { text: "" } })
    assert.equal(p.prompts.length, 0)
    assert.equal(p.synthetic.length, 1)
    assert.match(p.synthetic[0].text, /nothing to review/)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("setup works when ctx.command is missing", async () => {
  const fx = fixture()
  const p = await start(fx)
  try {
    assert.equal(typeof p.cleanup, "function")
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

function logCtx(w) {
  const logged = []
  w.ctx.log = { error: (entry) => void logged.push(entry) }
  return logged
}

test("/learn-check failure is logged unconditionally and reported to the session", async () => {
  const fx = fixture()
  const w = withCommands(fx)
  const logged = logCtx(w)
  const texts = []
  let first = true
  w.ctx.session.synthetic = async (input) => {
    if (first) {
      first = false
      throw new Error("synthetic broke")
    }
    texts.push(input.text)
  }
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store })
  try {
    candidate(fx, "work", "a.b", "proposed")
    await w.commands["learn-check"].execute({ sessionID: "ses_1", prompt: { text: "" } })
    assert.ok(logged.some((e) => e.message === "learn-check failed" && /synthetic broke/.test(e.error)), JSON.stringify(logged))
    assert.equal(texts.length, 1)
    assert.match(texts[0], /\/learn-check failed: synthetic broke/)
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("/learn-check and /learn-review never throw when synthetic/prompt are missing or throwing", async () => {
  const fx = fixture()
  candidate(fx, "work", "a.b", "proposed")
  const w = withCommands(fx)
  const logged = logCtx(w)
  delete w.ctx.session.synthetic
  delete w.ctx.session.prompt
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store })
  try {
    await w.commands["learn-check"].execute({ sessionID: "ses_1" })
    await w.commands["learn-review"].execute({ sessionID: "ses_1" })
    assert.ok(logged.some((e) => e.message === "learn-check failed"))
    assert.ok(logged.some((e) => e.message === "learn-review failed"))

    // Everything throws, including the error report itself.
    w.ctx.session.synthetic = async () => {
      throw new Error("s")
    }
    w.ctx.session.prompt = async () => {
      throw new Error("p")
    }
    await w.commands["learn-check"].execute({ sessionID: "ses_1" })
    await w.commands["learn-review"].execute({ sessionID: "ses_1" })
    assert.ok(logged.some((e) => e.message === "learn-review error report failed"), JSON.stringify(logged))
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("/learn-review prompt failure posts a short error to the session", async () => {
  const fx = fixture()
  candidate(fx, "work", "a.b", "proposed")
  const w = withCommands(fx)
  const logged = logCtx(w)
  w.ctx.session.prompt = async () => {
    throw new Error("prompt broke")
  }
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store })
  try {
    await w.commands["learn-review"].execute({ sessionID: "ses_1", prompt: { text: "" } })
    assert.ok(logged.some((e) => e.message === "learn-review failed"))
    assert.match(w.synthetic[0].text, /\/learn-review failed: prompt broke/)
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("/learn-check distinguishes missing store from store without candidates", async () => {
  const fx = fixture({ learnings: false })
  const p = await startCommands(fx)
  try {
    await p.commands["learn-check"].execute({ sessionID: "ses_1" })
    assert.match(p.synthetic[0].text, /has no profiles with candidates yet/)
    assert.doesNotMatch(p.synthetic[0].text, /no store at/)
  } finally {
    p.cleanup()
  }
  const missing = { ...fx, store: join(fx.root, "nope") }
  const q = await startCommands(missing)
  try {
    await q.commands["learn-check"].execute({ sessionID: "ses_1" })
    assert.match(q.synthetic[0].text, /^self-improving: no store at /)
  } finally {
    q.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("backup spawns via bash and logs the first spawn error once, unconditionally", async () => {
  const fx = fixture()
  const calls = []
  const logged = []
  const errSpawn = (command, args, options) => {
    calls.push({ command, args })
    const handlers = {}
    setTimeout(() => handlers.error?.(new Error("spawn EACCES")), 5)
    return { on: (event, fn) => void (handlers[event] = fn) }
  }
  const fake = fakeCtx(fx)
  fake.ctx.log = { error: (entry) => void logged.push(entry) }
  const cleanup = await plugin.setup(fake.ctx, { spawn: errSpawn, storeRoot: fx.store })
  try {
    fake.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } })
    await tick()
    fake.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } })
    await tick()
    assert.equal(calls.length, 2)
    assert.equal(calls[0].command, "bash")
    assert.match(calls[0].args[0], /bin\/learn$/)
    assert.equal(calls[0].args[1], "backup")
    const errs = logged.filter((e) => e.message === "backup spawn error")
    assert.equal(errs.length, 1, JSON.stringify(logged))
    assert.match(errs[0].error, /EACCES/)
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("backup survives a synchronously throwing spawn", async () => {
  const fx = fixture()
  const logged = []
  const fake = fakeCtx(fx)
  fake.ctx.log = { error: (entry) => void logged.push(entry) }
  const cleanup = await plugin.setup(fake.ctx, {
    spawn: () => {
      throw new Error("ENOENT")
    },
    storeRoot: fx.store,
  })
  try {
    fake.emit({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } })
    await tick()
    assert.ok(logged.some((e) => e.message === "backup spawn error" && /ENOENT/.test(e.error)), JSON.stringify(logged))
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("context scan is cached: unchanged files are not re-read, changes invalidate", async () => {
  const fx = fixture()
  const inbox = join(fx.store, "work", "inbox")
  const cands = join(fx.store, "work", "candidates")
  mkdirSync(cands, { recursive: true })
  const inboxFile = join(inbox, "2026-02-01.jsonl")
  const candFile = join(cands, "test.a.md")
  const candBody = (status) => `---\nstatus: ${status}\n---\nx\n`
  writeFileSync(inboxFile, `{"ts":"2026-02-01T00:00:00Z"}\n`)
  writeFileSync(candFile, candBody("proposed"))
  writeFileSync(join(fx.store, "work", "INDEX.md"), "- [a.b](candidates/a.b.md) — correction ×1: d\n")
  const fixed = new Date("2026-02-01T00:00:00Z")
  for (const path of [inboxFile, candFile]) utimesSync(path, fixed, fixed)
  const p = await start(fx)
  const run = async () => {
    const ev = { sessionID: "ses_1", system: [] }
    await p.hooks["session:context"](ev)
    return ev.system[0]?.text ?? ""
  }
  // Rewrite with same size and the same mtime: a re-read would change the result.
  const sneak = (path, content) => {
    writeFileSync(path, content)
    utimesSync(path, fixed, fixed)
  }
  try {
    const first = await run()
    assert.ok(first.includes("Inbox: 1 unprocessed event."), first)
    assert.ok(first.includes("(1): test.a"), first)

    sneak(inboxFile, " ".repeat(29) + "\n")
    sneak(candFile, candBody("rejected "))
    sneak(candFile, candBody("proposed"))
    assert.equal(await run(), first)

    // Real changes (new size/mtime) invalidate.
    writeFileSync(inboxFile, `{"ts":"2026-02-01T00:00:00Z"}\n{"ts":"2026-02-01T00:00:01Z"}\n`)
    writeFileSync(candFile, candBody("candidate"))
    const second = await run()
    assert.ok(second.includes("Inbox: 2 unprocessed events."), second)
    assert.ok(!second.includes("Proposed, waiting"), second)

    // New files and removals are picked up.
    writeFileSync(join(cands, "test.b.md"), candBody("proposed"))
    rmSync(inboxFile)
    const third = await run()
    assert.ok(third.includes("(1): test.b"), third)
    assert.ok(!third.includes("Inbox:"), third)
  } finally {
    p.cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

function skillFile(root, description = "Test desc") {
  const path = join(root, "SKILL.md")
  writeFileSync(path, `---\nname: self-improving\ndescription: ${description}\n---\n\n# Body\ntext\n`)
  return path
}

function skillCtx(fx, { existing = false, reg = { disposed: 0 } } = {}) {
  const fake = fakeCtx(fx)
  const added = []
  fake.ctx.skill = {
    transform: async (fn) => {
      fn({ get: (id) => (existing && id === "self-improving" ? {} : undefined), add: (info) => void added.push(info) })
      return { dispose: () => void (reg.disposed += 1) }
    },
  }
  return { ...fake, added, reg }
}

test("registers the bundled skill via ctx.skill.transform and disposes it", async () => {
  const fx = fixture()
  const w = skillCtx(fx)
  const path = skillFile(fx.root)
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store, skillPath: path })
  try {
    assert.equal(w.added.length, 1)
    assert.deepEqual(w.added[0], { id: "self-improving", name: "self-improving", description: "Test desc", path, content: "# Body\ntext" })
  } finally {
    cleanup()
    assert.equal(w.reg.disposed, 1)
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("bundled SKILL.md registers with real frontmatter stripped", async () => {
  const fx = fixture()
  const w = skillCtx(fx)
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store })
  try {
    assert.equal(w.added.length, 1)
    assert.ok(w.added[0].description.length > 10)
    assert.ok(!w.added[0].content.startsWith("---"))
    assert.ok(w.added[0].content.startsWith("# Self-Improving"))
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("skips skill registration when self-improving already exists", async () => {
  const fx = fixture()
  const w = skillCtx(fx, { existing: true })
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store, skillPath: skillFile(fx.root) })
  try {
    assert.equal(w.added.length, 0)
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})

test("skill registration failures never throw", async () => {
  const fx = fixture()
  const w = fakeCtx(fx)
  w.ctx.skill = { transform: async () => { throw new Error("boom") } }
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store, skillPath: skillFile(fx.root) })
  cleanup()
  const w2 = skillCtx(fx)
  const cleanup2 = await plugin.setup(w2.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store, skillPath: join(fx.root, "missing.md") })
  cleanup2()
  assert.equal(w2.added.length, 0)
  rmSync(fx.root, { recursive: true, force: true })
})

test("injection and /learn-review name the absolute CLI path", async () => {
  const fx = fixture()
  writeFileSync(join(fx.store, "work", "INDEX.md"), "# Index\n- [a.b](candidates/a.b.md) — correction ×2: x\n")
  const w = withCommands(fx)
  const cleanup = await plugin.setup(w.ctx, { spawn: fakeSpawn([]), storeRoot: fx.store, learnCli: "/abs/bin/learn" })
  try {
    candidate(fx, "work", "a.b", "proposed")
    await w.commands["learn-review"].execute({ sessionID: "ses_1", prompt: { text: "" } })
    assert.match(w.prompts[0].text, /\/abs\/bin\/learn --profile/)
    const ev = { sessionID: "ses_1", system: [] }
    await w.hooks["session:context"](ev)
    assert.match(ev.system[0].text, /run it as `\/abs\/bin\/learn`/)
  } finally {
    cleanup()
    rmSync(fx.root, { recursive: true, force: true })
  }
})
