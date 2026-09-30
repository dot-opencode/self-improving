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

// Returns an array of error strings; empty means the markers file is valid.
export function validateMarkers(json) {
  const errors = []
  if (!json || typeof json !== "object" || Array.isArray(json)) return ["markers must be a JSON object"]
  if (json.version !== 1) errors.push(`unsupported version: ${JSON.stringify(json.version)} (expected 1)`)
  const lists = [["confirmation", json.confirmation]]
  if (!json.correction || typeof json.correction !== "object") errors.push("missing 'correction' object")
  else for (const tier of ["strong", "medium", "weak"]) lists.push([`correction.${tier}`, json.correction[tier]])
  for (const [name, list] of lists) {
    if (list === undefined) continue
    if (!Array.isArray(list)) { errors.push(`'${name}' must be an array`); continue }
    list.forEach((item, i) => {
      if (typeof item !== "string" || !item) { errors.push(`'${name}[${i}]' must be a non-empty string`); return }
      try { new RegExp(unicodeize(item), "iu") } catch (error) { errors.push(`'${name}[${i}]' invalid regex ${JSON.stringify(item)}: ${error.message}`) }
    })
  }
  return errors
}

// Default first, personal appended. Missing/invalid files are ignored.
export function loadMarkers(defaultPath, personalPath) {
  return compileMarkers(readJson(defaultPath), readJson(personalPath))
}

// `opencode run "..."` and pasted quotes wrap the whole message; anchors must see the real start.
export function stripWrappingQuotes(text) {
  const value = String(text ?? "").trim()
  const wrapped = value.match(/^(["'«„“])([\s\S]*)(["'»“”])$/)
  return wrapped ? wrapped[2].trim() : value
}

export function normalize(text) {
  return stripWrappingQuotes(text).slice(0, MATCH_WINDOW).toLowerCase()
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
  // Confirmations are matched (for `learn markers test`) but never captured by the plugin.
  if (match.role === "confirmation") return false
  if (isFirstMessage || isSubagent) return false
  const value = String(text ?? "").trim()
  if (!value || value.length >= MATCH_WINDOW) return false
  if (/https?:\/\//i.test(value) || value.includes("/Users/") || value.includes("/home/")) return false
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
  [/Basic\s+[A-Za-z0-9+/=]{8,}/g, "[redacted]"],
  [/\b(github_pat_|glpat-|gh[opsur]_)\w+/g, "[redacted]"],
  [/eyJ[\w-]{10,}\.[\w-]{10,}\.[\w-]+/g, "[redacted]"],
  [/AKIA[0-9A-Z]{16}/g, "[redacted]"],
  [/xox[baprs]-\S+/g, "[redacted]"],
  [/:\/\/[^/\s:]+:[^@\s]+@/g, "://[redacted]@"],
  [/"?(api[_-]?key|token|secret|password|passwd)"?\s*[=:]\s*"?[^"\s,}]+"?/gi, "[redacted]"],
  [/\/(?:Users|home)\/[^/\s]+/g, "~"],
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

const EXIT_RE = /exit(?:ed with)? code:? [1-9]\d*/i
const BASH_TOOLS = new Set(["bash", "shell"])
const BASH_PATTERNS = [EXIT_RE, /^Error:|Traceback|FAILED|npm ERR!/m]
const EDIT_TOOLS = new Set(["edit", "patch", "write", "multiedit", "apply_patch"])
const EDIT_PATTERNS = [/not found|oldString/]

function lastNonEmptyLine(text) {
  const lines = String(text ?? "").split("\n").map((line) => line.trim()).filter(Boolean)
  return lines[lines.length - 1] ?? ""
}

function failureHead(text, re) {
  const lines = text.split("\n").map((line) => line.trim()).filter(Boolean)
  const hit = lines.find((line) => re.test(line)) ?? lines[0] ?? ""
  if (/Traceback/.test(hit)) {
    const last = lines[lines.length - 1] ?? ""
    return last && last !== hit ? `${hit}: ${last}` : hit
  }
  if (hit.length >= 20) return hit
  const other = [...lines].reverse().find((line) => line !== hit)
  return other ? `${hit}: ${other}` : hit
}

// Structured bash result: `{ exit, truncated, output }`, possibly nested under
// `result.output` or `result.metadata`.
function numericExit(result) {
  if (!result || typeof result !== "object") return undefined
  // Observed on OpenCode v2.0.20: result.metadata.metadata.exit (shell tool).
  for (const candidate of [result.exit, result.output?.exit, result.metadata?.exit, result.metadata?.metadata?.exit]) {
    if (Number.isInteger(candidate)) return candidate
  }
  return undefined
}

export function detectToolFailure(ev) {
  if (!ev || typeof ev !== "object") return undefined
  const tool = ev.tool
  if (ev.status === "error") {
    const head = ev.error?.message ?? (typeof ev.error === "string" ? ev.error : "tool error")
    return { tool, head: String(head) }
  }
  if (ev.status !== "completed") return undefined
  const name = String(tool ?? "").toLowerCase()
  const isBash = BASH_TOOLS.has(name)
  if (!isBash && !EDIT_TOOLS.has(name)) return undefined

  const result = ev.result
  const text = typeof result === "string"
    ? result
    : [flatten(result?.content), flatten(result?.output), flatten(result?.metadata?.output)].filter(Boolean).join("\n")

  if (isBash) {
    const exit = numericExit(result)
    if (exit !== undefined && exit !== 0) {
      const last = lastNonEmptyLine(text)
      return { tool, head: last ? `exit ${exit}: ${last}` : `exit ${exit}` }
    }
    // A known zero exit is authoritative: never pattern-scan successful output
    // (it may legitimately contain "Traceback", "Error:", or an "exit code" string).
    if (exit === 0) return undefined
  }
  if (!text) return undefined
  for (const re of isBash ? BASH_PATTERNS : EDIT_PATTERNS) {
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
    "Triage with the self-improving skill at a natural pause (end of task or right after a user correction), not mid-task.",
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
