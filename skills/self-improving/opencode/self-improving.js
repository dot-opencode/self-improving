import { spawn as nodeSpawn } from "node:child_process"
import { realpathSync } from "node:fs"
import { appendFile, mkdir, readdir, readFile, readlink } from "node:fs/promises"
import { homedir } from "node:os"
import { basename, dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import {
  OPEN,
  buildInjection,
  detectToolFailure,
  inboxLine,
  loadMarkers,
  matchMarkers,
  redact,
  shouldCapture,
} from "./learn-core.js"

let moduleDirectory = dirname(fileURLToPath(import.meta.url))
try {
  moduleDirectory = realpathSync(moduleDirectory)
} catch {
  // Fall back to the unresolved path.
}
const learnCli = resolve(moduleDirectory, "..", "bin", "learn")
const defaultMarkersPath = join(moduleDirectory, "markers.default.json")

const BACKUP_EVENTS = new Set(["session.execution.succeeded", "session.execution.failed", "session.execution.interrupted"])
const INERT_RECHECK_MS = 10_000

const debugEnabled = () => process.env.SELF_IMPROVING_DEBUG === "1"

function timestamp() {
  return new Date().toISOString().replace(/\.\d+Z$/, "Z")
}

// A plain V2 plugin object ({ id, setup }); no imports from @opencode packages so
// it stays loadable when installed as a symlink with no node_modules nearby.
export default {
  id: "self-improving",
  async setup(ctx, deps = {}) {
    const spawn = deps.spawn ?? nodeSpawn
    const storeRoot = deps.storeRoot ?? process.env.AGENT_LEARNINGS_HOME ?? join(homedir(), ".agent-learnings")
    const cli = deps.learnCli ?? learnCli
    const controller = new AbortController()
    const sessions = new Map()
    const backups = new Map()

    function debug(message, extra) {
      if (!debugEnabled()) return
      try {
        if (ctx.log?.error) void Promise.resolve(ctx.log.error({ service: "self-improving", message, ...extra })).catch(() => {})
        else console.error("[self-improving]", message, extra ?? "")
      } catch {
        // Logging must not interrupt OpenCode hooks.
      }
    }

    // Per-session state. Returns undefined when the session is inert
    // (no `.learnings` symlink in its directory).
    async function sessionState(sessionID) {
      if (!sessionID) return undefined
      const cached = sessions.get(sessionID)
      if (cached && (!cached.inert || Date.now() - cached.checkedAt < INERT_RECHECK_MS)) {
        return cached.inert ? undefined : cached
      }
      const info = await ctx.session.get({ sessionID })
      const directory = info?.location?.directory ?? ctx.location?.directory
      const state = cached ?? {
        inert: true,
        checkedAt: 0,
        userCount: 0,
        seq: 0,
        lastToolSeq: 0,
        lastPromptSeq: 0,
        lastTool: undefined,
        heads: new Set(),
        messages: new Set(),
        texts: new Set(),
      }
      state.checkedAt = Date.now()
      state.directory = directory
      state.isSubagent = Boolean(info?.parentID)
      state.project = directory ? basename(directory) : undefined
      state.inert = true
      if (directory) {
        try {
          const target = await readlink(join(directory, ".learnings"))
          const profile = basename(target)
          if (profile) {
            state.profile = profile
            state.dir = join(storeRoot, profile)
            state.inert = false
          }
        } catch {
          // No `.learnings` symlink: stay inert.
        }
      }
      sessions.set(sessionID, state)
      return state.inert ? undefined : state
    }

    async function writeEvent(state, event) {
      const inbox = join(state.dir, "inbox")
      await mkdir(inbox, { recursive: true })
      await appendFile(join(inbox, `${event.ts.slice(0, 10)}.jsonl`), inboxLine(event))
    }

    async function onToolAfter(ev) {
      try {
        debug("execute.after", { status: ev?.status, tool: ev?.tool })
        const state = await sessionState(ev?.sessionID)
        if (!state) return
        state.lastToolSeq = ++state.seq
        state.lastTool = ev.tool
        const failure = detectToolFailure(ev)
        if (!failure) return
        const head = redact(failure.head)
        if (!head || state.heads.has(head)) return
        state.heads.add(head)
        await writeEvent(state, {
          ts: timestamp(),
          kind: "failure",
          tool: failure.tool,
          head,
          project: state.project,
          session: ev.sessionID,
        })
      } catch (error) {
        debug("execute.after failed", { error: String(error) })
      }
    }

    async function onPrompt(ev) {
      try {
        const sessionID = ev?.sessionID
        const state = await sessionState(sessionID)
        if (!state) return
        if (ev.messageID) {
          if (state.messages.has(ev.messageID)) return
          state.messages.add(ev.messageID)
        }
        const text = typeof ev.prompt?.text === "string" ? ev.prompt.text : ""
        state.userCount += 1
        const isFirstMessage = state.userCount === 1
        const prevWasToolAction = state.lastToolSeq > state.lastPromptSeq
        const prevTool = state.lastTool
        state.lastPromptSeq = ++state.seq
        debug("prompt", { session: sessionID, first: isFirstMessage, length: text.length })

        const markers = loadMarkers(defaultMarkersPath, join(storeRoot, "markers.json"))
        const match = matchMarkers(text, markers)
        if (!shouldCapture({ text, isFirstMessage, isSubagent: state.isSubagent, prevWasToolAction, match })) return

        const verbatim = text.trim().toLowerCase()
        if (state.texts.has(verbatim)) return
        state.texts.add(verbatim)

        const event = {
          ts: timestamp(),
          kind: match.role,
          tier: match.tier,
          marker: match.marker,
          snippet: redact(text),
          project: state.project,
          session: sessionID,
        }
        if (prevWasToolAction && prevTool) event.prev_tool = prevTool
        await writeEvent(state, event)
      } catch (error) {
        debug("prompt failed", { error: String(error) })
      }
    }

    async function unackedCount(state) {
      const inbox = join(state.dir, "inbox")
      let ack = 0
      try {
        const parsed = Date.parse((await readFile(join(inbox, ".ack"), "utf8")).trim())
        if (!Number.isNaN(parsed)) ack = parsed
      } catch {
        // No ack marker: everything is unprocessed.
      }
      let files
      try {
        files = (await readdir(inbox)).filter((name) => name.endsWith(".jsonl"))
      } catch {
        return 0
      }
      let count = 0
      for (const name of files) {
        let content
        try {
          content = await readFile(join(inbox, name), "utf8")
        } catch {
          continue
        }
        for (const line of content.split("\n")) {
          if (!line.trim()) continue
          try {
            const ts = Date.parse(JSON.parse(line).ts)
            if (Number.isNaN(ts) || ts > ack) count += 1
          } catch {
            // Skip malformed lines.
          }
        }
      }
      return count
    }

    async function onContext(ev) {
      try {
        if (!Array.isArray(ev?.system)) return
        if (ev.system.some((part) => typeof part?.text === "string" && part.text.includes(OPEN))) return
        const state = await sessionState(ev.sessionID)
        if (!state) return
        let index = ""
        try {
          index = await readFile(join(state.dir, "INDEX.md"), "utf8")
        } catch {
          // No INDEX yet.
        }
        const text = buildInjection(index, await unackedCount(state))
        if (!text) return
        if (ev.system.some((part) => typeof part?.text === "string" && part.text.includes(OPEN))) return
        ev.system.push({ type: "text", text })
      } catch (error) {
        debug("context failed", { error: String(error) })
      }
    }

    function runBackup(directory) {
      return new Promise((done) => {
        try {
          const child = spawn(cli, ["backup"], { cwd: directory, stdio: "ignore" })
          if (!child || typeof child.on !== "function") return done()
          child.on("error", (error) => {
            debug("backup spawn error", { error: String(error), directory })
            done()
          })
          child.on("close", (code) => {
            if (code) debug("backup exited nonzero", { code, directory })
            done()
          })
        } catch (error) {
          debug("backup spawn failed", { error: String(error), directory })
          done()
        }
      })
    }

    async function backup(sessionID) {
      try {
        const state = await sessionState(sessionID)
        if (!state || controller.signal.aborted) return
        const directory = state.directory
        if (backups.has(directory)) return
        const pending = runBackup(directory).finally(() => backups.delete(directory))
        backups.set(directory, pending)
        await pending
      } catch (error) {
        debug("backup failed", { error: String(error), sessionID })
      }
    }

    try {
      await ctx.tool.hook("execute.after", onToolAfter)
      await ctx.session.hook("prompt", onPrompt)
      await ctx.session.hook("context", onContext)
    } catch (error) {
      debug("hook registration failed", { error: String(error) })
    }

    void (async () => {
      for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
        if (controller.signal.aborted) break
        if (!event) continue
        debug("event", { type: event.type })
        if (!BACKUP_EVENTS.has(event.type)) continue
        const sessionID = event.data?.sessionID
        if (sessionID) void backup(sessionID)
      }
    })().catch((error) => debug("event subscription failed", { error: String(error) }))

    return () => controller.abort()
  },
}
