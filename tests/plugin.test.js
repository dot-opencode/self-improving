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
  let wake
  const ctx = {
    tool: { hook: async (name, fn) => void (hooks[`tool:${name}`] = fn) },
    session: {
      hook: async (name, fn) => void (hooks[`session:${name}`] = fn),
      get: async ({ sessionID }) => ({
        parentID: sessionID === "ses_sub" ? "ses_parent" : undefined,
        location: { directory: fx.project },
      }),
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
    assert.ok(text.includes("1 unprocessed event "), text.slice(-300))
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
