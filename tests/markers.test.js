import assert from "node:assert/strict"
import { dirname, join } from "node:path"
import { test } from "node:test"
import { fileURLToPath } from "node:url"

import { loadMarkers, matchMarkers, shouldCapture } from "../skills/self-improving/opencode/learn-core.js"

const dir = join(dirname(fileURLToPath(import.meta.url)), "..", "skills", "self-improving", "opencode")
const markers = loadMarkers(join(dir, "markers.default.json"), join(dir, "markers.personal.default.json"))
const defaultOnly = loadMarkers(join(dir, "markers.default.json"))

const tierOf = (text) => matchMarkers(text, markers)?.tier

test("strong RU/EN positives", () => {
  for (const text of [
    "Нет, я про другое",
    "нет. не так",
    "Стой",
    "стоп, что ты делаешь",
    "Откати это",
    "верни как было",
    "Я же сказал не трогать",
    "это не тот файл",
    "сделано не так",
    "No, use the other one",
    "Stop",
    "wait a second",
    "please revert that",
    "what are you doing",
    "I said use pnpm",
  ]) {
    assert.equal(tierOf(text), "strong", text)
  }
})

test("medium RU/EN positives", () => {
  for (const text of [
    "Не делай так",
    "не трогай конфиг",
    "тут не должно быть моков",
    "я про тесты",
    "всё ещё падает",
    "до сих пор красный",
    "не помогло",
    "Зачем ты это добавил",
    "а зачем нам это",
    "мы же договаривались",
    "Опять сломал",
    "Don't touch that",
    "do not add deps",
    "Instead use fetch",
    "it's still broken",
    "Actually, use tabs",
    "Why did you delete it",
    "Again the same error",
    "from now on run lint first",
    "Always run tests",
    "never commit .env",
  ]) {
    assert.equal(tierOf(text), "medium", text)
  }
})

test("weak RU/EN positives", () => {
  for (const text of [
    "Почему ci упал?",
    "а почему так",
    "зачем",
    "используй X вместо Y",
    "убери это",
    "так лучше",
    "только тесты",
    "why?",
    "use A instead of B",
    "remove the log",
    "just the tests",
  ]) {
    assert.equal(tierOf(text), "weak", text)
  }
})

test("confirmation is not a correction", () => {
  for (const text of ["ок", "Окей", "да", "давай", "давай", "ok", "yes", "продолжай", "готово", "делай", "LGTM", "1"]) {
    const match = matchMarkers(text, markers)
    assert.equal(match?.role, "confirmation", text)
    assert.equal(match.tier, "confirmation")
  }
  assert.equal(matchMarkers("давай", markers).role, "confirmation")
})

test("negatives: no match", () => {
  for (const text of ["нет нагрузки", "нетривиальная задача", "добавь тест для парсера", "", "   ", "12", "отлично"]) {
    assert.equal(matchMarkers(text, markers), undefined, text)
  }
})

test("personal markers are appended after defaults", () => {
  assert.equal(matchMarkers("Нет, я имею в виду другое", defaultOnly)?.tier, "strong")
  assert.equal(matchMarkers("на русском пожалуйста", defaultOnly), undefined)
  assert.equal(matchMarkers("на русском пожалуйста", markers)?.tier, "medium")
  assert.equal(matchMarkers("спроси меня сначала", markers)?.tier, "weak")
  assert.equal(matchMarkers("бляха", markers), undefined)
  assert.equal(matchMarkers("блять", markers)?.tier, "strong")
})

test("strongest tier wins and marker source is reported", () => {
  const match = matchMarkers("Нет, не делай так", markers)
  assert.equal(match.role, "correction")
  assert.equal(match.tier, "strong")
  assert.equal(match.marker, "^нет[,.\\-—]")
})

test("missing marker files yield empty markers", () => {
  const empty = loadMarkers("/nonexistent/a.json", "/nonexistent/b.json")
  assert.equal(matchMarkers("Нет, я про другое", empty), undefined)
})

const capture = (text, opts = {}) => {
  const match = matchMarkers(text, markers)
  return shouldCapture({ text, isFirstMessage: false, isSubagent: false, prevWasToolAction: false, match, ...opts })
}

test("shouldCapture accepts follow-up corrections", () => {
  assert.equal(capture("Нет, я про другое"), true)
  assert.equal(capture("ок"), true)
})

test("shouldCapture rejects first message, subagent, no match", () => {
  assert.equal(capture("Нет, я про другое", { isFirstMessage: true }), false)
  assert.equal(capture("Нет, я про другое", { isSubagent: true }), false)
  assert.equal(shouldCapture({ text: "x", isFirstMessage: false, isSubagent: false, match: undefined }), false)
})

test("shouldCapture rejects URLs, /Users/ paths, pasted logs", () => {
  assert.equal(capture("Нет, смотри https://example.com/x"), false)
  assert.equal(capture("нет, файл /Users/me/x.txt"), false)
  const logs = "нет, вот лог\nat foo (a.js:1)\nat bar (b.js:2)\nat baz (c.js:3)"
  assert.equal(capture(logs), false)
  assert.equal(capture("нет, вот лог\nat foo (a.js:1)"), true)
})

test("shouldCapture rejects >=400 chars", () => {
  assert.equal(capture(`Нет, ${"а".repeat(400)}`), false)
  assert.equal(capture(`Нет, ${"а".repeat(100)}`), true)
})

test("weak tier requires prevWasToolAction and len<200", () => {
  assert.equal(capture("Почему ci упал?"), false)
  assert.equal(capture("Почему ci упал?", { prevWasToolAction: true }), true)
  assert.equal(capture(`почему ${"а".repeat(200)}`, { prevWasToolAction: true }), false)
})
