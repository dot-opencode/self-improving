import { readFileSync } from "node:fs"

// Pure(ish) helpers for the self-improving plugin: marker matching, capture
// filters, redaction, tool-failure detection, injection block. Nothing here
// touches OpenCode; everything is unit-testable.

export const OPEN = "<self-improving>"
export const CLOSE = "</self-improving>"
export const INJECTION_LIMIT = 4096
export const MATCH_WINDOW = 400

const WORD = "\\p{L}\\p{N}_"

// JS `\b` and `\w` are ASCII-only even with the `u` flag, which breaks every
// Cyrillic marker like `\bя про\b`. Rewrite them to Unicode-aware equivalents
// before compiling. Marker JSON keeps the plain regex strings.
export function unicodeize(source) {
  let out = ""
  let inClass = false
  for (let i = 0; i < source.length; i++) {
    const c = source[i]
    if (c === "\\") {
      const n = source[i + 1]
      if (n === undefined) {
        out += c
        break
      }
      i++
      if (!inClass && n === "b") out += `(?:(?<=[${WORD}])(?![${WORD}])|(?<![${WORD}])(?=[${WORD}]))`
      else if (!inClass && n === "B") out += `(?:(?<=[${WORD}])(?=[${WORD}])|(?<![${WORD}])(?![${WORD}]))`
      else if (n === "w") out += inClass ? WORD : `[${WORD}]`
      else if (!inClass && n === "W") out += `[^${WORD}]`
      else out += c + n
      continue
    }
    if (c === "[" && !inClass) inClass = true
    else if (c === "]" && inClass) inClass = false
    out += c
  }
  return out
}

function compileList(list) {
  const compiled = []
  if (!Array.isArray(list)) return compiled
  for (const source of list) {
    if (typeof source !== "string" || source === "") continue
    try {
      compiled.push({ source, re: new RegExp(unicodeize(source), "iu") })
    } catch {
      // Skip invalid user-provided regex; `learn doctor` is the place to flag it.
    }
  }
  return compiled
}

function readJson(path) {
  if (!path) return undefined
  try {
    return JSON.parse(readFileSync(path, "utf8"))
  } catch {
    return undefined
  }
}

export function compileMarkers(...sources) {
  const merged = { correction: { strong: [], medium: [], weak: [] }, confirmation: [] }
  for (const source of sources) {
    if (!source || typeof source !== "object") continue
    for (const tier of ["strong", "medium", "weak"]) {
      merged.correction[tier].push(...compileList(source.correction?.[tier]))
    }
    merged.confirmation.push(...compileList(source.confirmation))
  }
  return merged
}

// Default first, personal appended. Missing/invalid files are ignored.
export function loadMarkers(defaultPath, personalPath) {
  return compileMarkers(readJson(defaultPath), readJson(personalPath))
}

export function normalize(text) {
  return String(text ?? "").trim().slice(0, MATCH_WINDOW).toLowerCase()
}

// Order: strong → medium → weak → confirmation.
export function matchMarkers(text, markers) {
  if (!markers) return undefined
  const value = normalize(text)
  if (!value) return undefined
  for (const tier of ["strong", "medium", "weak"]) {
    for (const { source, re } of markers.correction?.[tier] ?? []) {
      if (re.test(value)) return { role: "correction", tier, marker: source }
    }
  }
  for (const { source, re } of markers.confirmation ?? []) {
    if (re.test(value)) return { role: "confirmation", tier: "confirmation", marker: source }
  }
  return undefined
}

const LOG_LINE = /^\s*(at |Traceback|Error|\d{4}-\d{2}-\d{2}T)/

export function shouldCapture({ text, isFirstMessage, isSubagent, prevWasToolAction, match } = {}) {
  if (!match) return false
  if (isFirstMessage || isSubagent) return false
  const value = String(text ?? "").trim()
  if (!value || value.length >= MATCH_WINDOW) return false
  if (/https?:\/\//i.test(value) || value.includes("/Users/")) return false
  const logLines = value.split("\n").filter((line) => LOG_LINE.test(line)).length
  if (logLines >= 3) return false
  if (match.tier === "weak") {
    if (!prevWasToolAction || value.length >= 200) return false
  }
  return true
}

const REDACTIONS = [
  [/Bearer\s+\S+/g, "[redacted]"],
  [/sk-[A-Za-z0-9_-]{8,}/g, "[redacted]"],
  [/ghp_\w+/g, "[redacted]"],
  [/AKIA[0-9A-Z]{16}/g, "[redacted]"],
  [/xox[baprs]-\S+/g, "[redacted]"],
  [/:\/\/[^/\s:]+:[^@\s]+@/g, "://[redacted]@"],
  [/(api[_-]?key|token|secret|password)\s*[=:]\s*\S+/gi, "[redacted]"],
]

export function redact(text, max = 300) {
  let value = String(text ?? "")
  for (const [re, replacement] of REDACTIONS) value = value.replace(re, replacement)
  value = value.replace(/\s+/g, " ").trim()
  if (value.length > max) value = `${value.slice(0, Math.max(0, max - 1))}…`
  return value
}

function flatten(value, depth = 0) {
  if (value == null || depth > 4) return ""
  if (typeof value === "string") return value
  if (Array.isArray(value)) return value.map((item) => flatten(item, depth + 1)).filter(Boolean).join("\n")
  if (typeof value === "object") {
    if (typeof value.text === "string") return value.text
    return [value.content, value.output, value.stdout, value.stderr]
      .map((item) => flatten(item, depth + 1))
      .filter(Boolean)
      .join("\n")
  }
  return ""
}

const FAILURE_PATTERNS = [/exit code:? [1-9]/i, /oldString not found|not found in file/i, /^Error:|Traceback|FAILED|npm ERR!/m]

function failureHead(text, re) {
  const lines = text.split("\n").map((line) => line.trim()).filter(Boolean)
  const hit = lines.find((line) => re.test(line)) ?? lines[0] ?? ""
  if (hit.length >= 20) return hit
  const other = [...lines].reverse().find((line) => line !== hit)
  return other ? `${hit}: ${other}` : hit
}

export function detectToolFailure(ev) {
  if (!ev || typeof ev !== "object") return undefined
  const tool = ev.tool
  if (ev.status === "error") {
    const head = ev.error?.message ?? (typeof ev.error === "string" ? ev.error : "tool error")
    return { tool, head: String(head) }
  }
  if (ev.status !== "completed") return undefined
  const result = ev.result
  const text = typeof result === "string"
    ? result
    : [flatten(result?.content), flatten(result?.output)].filter(Boolean).join("\n")
  if (!text) return undefined
  for (const re of FAILURE_PATTERNS) {
    if (re.test(text)) return { tool, head: failureHead(text, re) }
  }
  return undefined
}

export function indexLines(indexText) {
  return String(indexText ?? "").split("\n").filter((line) => line.startsWith("- ["))
}

export function buildInjection(indexText, unackedCount = 0) {
  const lines = indexLines(indexText)
  const count = Number(unackedCount) || 0
  if (lines.length === 0 && count === 0) return undefined

  const head = `${OPEN}\nBehavior learnings for this project live in ./.learnings (managed by the \`learn\` CLI).`
  const tail = [
    `${count} unprocessed event${count === 1 ? "" : "s"} in .learnings/inbox.`,
    "Triage is done by the `self-improving` skill (`learn inbox`); do not act on these unprompted.",
    CLOSE,
  ].join("\n")
  const note = `[self-improving: candidates truncated at ${INJECTION_LIMIT} characters]`

  const build = (items, truncated) =>
    [head, `Candidates (${lines.length}):`, ...(items.length ? items : ["(none)"]), ...(truncated ? [note] : []), tail].join("\n")

  let text = build(lines, false)
  if (text.length <= INJECTION_LIMIT) return text

  const kept = []
  for (const line of lines) {
    if (build([...kept, line], true).length > INJECTION_LIMIT) break
    kept.push(line)
  }
  text = build(kept, true)
  return text.length <= INJECTION_LIMIT ? text : `${text.slice(0, INJECTION_LIMIT - CLOSE.length - 1)}\n${CLOSE}`
}

export function inboxLine(event) {
  return `${JSON.stringify(event)}\n`
}
