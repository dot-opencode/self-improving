// Checks of skills/self-improving/dream/{opencode.json,prompt.md} and of the
// placeholder substitution done by `learn dream --print-config`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SKILL = join(ROOT, "skills", "self-improving");
const LEARN = join(SKILL, "bin", "learn");
const template = JSON.parse(readFileSync(join(SKILL, "dream", "opencode.json"), "utf8"));
const prompt = readFileSync(join(SKILL, "dream", "prompt.md"), "utf8");

const shellRules = (perms) => perms.filter((p) => p.action === "shell");
const idx = (perms, action, resource) => perms.findIndex((p) => p.action === action && p.resource === resource);

// Glob -> RegExp: `*` matches any run of characters (including spaces, newlines), all else literal.
const globToRegExp = (glob) =>
  new RegExp("^" + glob.split("*").map((s) => s.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join("[\\s\\S]*") + "$");

// Last matching rule wins; no match means denied.
const shellDecision = (rules, command) => {
  let effect = "deny";
  for (const r of rules) if (globToRegExp(r.resource).test(command)) effect = r.effect;
  return effect;
};

test("template: default agent and agent set", () => {
  assert.equal(template.default_agent, "learn-dream");
  assert.deepEqual(Object.keys(template.agents).sort(), ["learn-dream", "learn-dream-check"]);
  assert.equal(template.agents["learn-dream"].mode, "primary");
  assert.equal(template.agents["learn-dream-check"].mode, "subagent");
  assert.doesNotMatch(JSON.stringify(template), /memory/i);
});

test("template: policies mirror the primary agent's permissions", () => {
  const fromPolicies = template.experimental.policies.map((p) => p.resource.replace(":", "\u0000"));
  const fromAgent = template.agents["learn-dream"].permissions
    .filter((p) => p.action !== "*")
    .map((p) => `${p.action}\u0000${p.resource}`);
  const pol = fromPolicies.filter((r) => !r.startsWith("*"));
  // Agent rules exclude the leading deny-all; the sets must otherwise match.
  assert.deepEqual([...pol].sort(), [...fromAgent].sort().filter((r) => !r.startsWith("*\u0000")));
});

test("template: deny-all first, then allows", () => {
  const perms = template.agents["learn-dream"].permissions;
  assert.deepEqual(perms[0], { action: "*", resource: "*", effect: "deny" });
  for (const a of ["read", "glob", "grep", "external_directory", "webfetch", "websearch"]) {
    assert.ok(idx(perms, a, "*") > 0, `${a} allowed`);
  }
  assert.equal(template.agents["learn-dream"].permissions.find((p) => p.action === "subagent").resource, "learn-dream-check");
});

test("template: learn commands allowed, dangerous ones denied after the allows", () => {
  const perms = template.agents["learn-dream"].permissions;
  const allow = [
    "dream --report*", "inbox*", "add *", "bump *", "ack *", "propose *", "reject *",
    "index*", "doctor*", "grep *", "status*", "markers list*", "markers test *",
  ];
  const deny = ["apply*", "init*", "backup*", "prune*", "markers add*", "install-opencode*"];
  const lastAllow = Math.max(...allow.flatMap((r) => [idx(perms, "shell", `__LEARN_BIN__ ${r}`), idx(perms, "shell", `rtk __LEARN_BIN__ ${r}`)]));
  for (const r of allow) {
    for (const pre of ["", "rtk "]) {
      const i = perms.findIndex((p) => p.action === "shell" && p.resource === `${pre}__LEARN_BIN__ ${r}` && p.effect === "allow");
      assert.ok(i > 0, `allow ${pre}${r}`);
    }
  }
  for (const r of deny) {
    for (const pre of ["", "rtk "]) {
      const i = perms.findIndex((p) => p.action === "shell" && p.resource === `${pre}__LEARN_BIN__ ${r}` && p.effect === "deny");
      assert.ok(i > lastAllow, `deny ${pre}${r} comes after every allow`);
    }
  }
});

test("template: edit only in the store, never .git or inbox", () => {
  const perms = template.agents["learn-dream"].permissions;
  const allowStore = idx(perms, "edit", "__STORE__/*");
  assert.ok(allowStore > 0);
  for (const r of ["__STORE__/.git*", "__STORE__/*/inbox/*", ".git", ".git/*", "*/.git", "*/.git/*"]) {
    const i = perms.findIndex((p) => p.action === "edit" && p.resource === r && p.effect === "deny");
    assert.ok(i > allowStore, `edit deny ${r} after the store allow`);
  }
});

test("template: git deny list, shell metacharacters and secret paths kept", () => {
  const perms = template.agents["learn-dream"].permissions;
  const shell = new Set(shellRules(perms).filter((p) => p.effect === "deny").map((p) => p.resource));
  for (const verb of ["push", "pull", "fetch", "remote", "config", "commit", "reset", "checkout", "rebase", "stash", "clean"]) {
    assert.ok(shell.has(`* ${verb}`) && shell.has(`* ${verb} *`), `deny git ${verb}`);
  }
  for (const m of [">", "|", ";", "&", "$(", "`"]) assert.ok(shell.has(`*${m}*`), `deny shell metachar ${m}`);
  const reads = new Set(perms.filter((p) => p.action === "read" && p.effect === "deny").map((p) => p.resource));
  for (const r of ["__HOME__/.ssh/*", "__HOME__/.aws/*", "__HOME__/.gnupg/*", "*.env", "*.env.*"]) assert.ok(reads.has(r), `deny read ${r}`);
});

test("template: check subagent is read-only", () => {
  const perms = template.agents["learn-dream-check"].permissions;
  assert.equal(perms.filter((p) => p.action === "edit").length, 0);
  assert.equal(shellRules(perms).filter((p) => p.effect === "allow" && /__LEARN_BIN__/.test(p.resource)).length, 0);
  assert.ok(shellRules(perms).some((p) => p.resource === "git log*" && p.effect === "allow"));
});

test("prompt: forbids applying and mentions the required pieces", () => {
  assert.match(prompt, /Never apply/);
  assert.match(prompt, /## Proposal/);
  assert.match(prompt, /git -C __STORE__ rm/);
  assert.match(prompt, /__LEARN_BIN__ dream --report/);
  assert.match(prompt, /__LEARN_BIN__ ack/);
  assert.match(prompt, /learn-dream-check/);
  assert.doesNotMatch(prompt, /__MEMORY_BIN__/);
});

test("prompt: draft protocol replaces free text in shell and learn grep", () => {
  assert.match(prompt, /__STORE__\/\.drafts\/draft\.md/);
  assert.match(prompt, /--from \.drafts\/draft\.md/);
  assert.match(prompt, /NEVER put free text/);
  assert.doesNotMatch(prompt, /__LEARN_BIN__ grep/);
  assert.doesNotMatch(prompt, /--evidence "/);
  assert.match(prompt, /grep tool on `__STORE__\/<p>\/candidates\/`/);
});

test("prompt: date field from the event ts, evidence without prefix", () => {
  assert.match(prompt, /`date:` is `YYYY-MM-DD` taken from the event's `ts`/);
  assert.match(prompt, /already prefixes the evidence line with `<date> <project>:`/);
  assert.match(prompt, /`evidence:` must be only the text/);
  assert.match(prompt, /`add`: key, kind, summary, evidence, project, behavior, owner, date/);
  assert.match(prompt, /`bump`: key, evidence, project, date/);
});

test("sandbox simulation: --from drafts and plain commands allowed, free text and apply denied", () => {
  const rendered = JSON.parse(
    JSON.stringify(template)
      .replaceAll("__LEARN_BIN__", "/opt/learn")
      .replaceAll("__STORE__", "/store")
      .replaceAll("__HOME__", "/home/u"),
  );
  const rules = shellRules(rendered.agents["learn-dream"].permissions);
  const bin = "/opt/learn";
  for (const pre of ["", "rtk "]) {
    for (const cmd of [
      `${bin} add --profile work --from .drafts/draft.md`,
      `${bin} bump --profile work --from .drafts/draft.md`,
      `${bin} reject --profile work --from .drafts/draft.md`,
      `${bin} ack --profile work --until 2026-09-30T12:00:00Z`,
      `${bin} inbox --profile work --limit 200`,
    ]) {
      assert.equal(shellDecision(rules, pre + cmd), "allow", `allowed: ${pre}${cmd}`);
    }
    for (const cmd of [
      `${bin} add --profile work --key git.x --kind failure --summary s --evidence "a; b"`,
      `${bin} add --profile work --key git.x --kind failure --summary s --evidence "did commit now"`,
      `${bin} add --profile work --from .drafts/draft.md; ls`,
      `${bin} apply --profile work git.x --owner skill:y --change-ref r`,
    ]) {
      assert.equal(shellDecision(rules, pre + cmd), "deny", `denied: ${pre}${cmd}`);
    }
  }
});

test("print-config substitutes every placeholder and stays valid JSON", () => {
  const tmp = realpathSync(mkdtempSync(join(tmpdir(), "learn-dream-")));
  try {
    const store = join(tmp, "st ore&x");
    mkdirSync(join(store, "work", "candidates"), { recursive: true });
    execFileSync("git", ["init", "-q", store]);
    const home = join(tmp, "home");
    mkdirSync(home);
    const out = execFileSync(LEARN, ["dream", "--print-config"], {
      env: { ...process.env, HOME: home, AGENT_LEARNINGS_HOME: store },
      encoding: "utf8",
    });
    assert.doesNotMatch(out, /__STORE__|__LEARN_BIN__|__HOME__/);
    const cfg = JSON.parse(out);
    const res = cfg.agents["learn-dream"].permissions.map((p) => p.resource);
    assert.ok(res.includes(`${store}/*`), "store substituted (with space and &)");
    assert.ok(res.includes(`${LEARN} dream --report*`), "learn bin substituted");
    assert.ok(res.includes(`${home}/.ssh/*`), "home substituted");
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
});
