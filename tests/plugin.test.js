import assert from "node:assert/strict"
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs"
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
    assert.deepEqual(p.calls[0].args, ["backup"])
    assert.match(p.calls[0].command, /bin\/learn$/)

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
