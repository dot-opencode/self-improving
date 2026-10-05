#!/usr/bin/env bash
# Self-contained scenario tests for skills/self-improving/bin/learn.
set -uo pipefail

ROOT="$(cd -P "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
LEARN="$ROOT/skills/self-improving/bin/learn"

TMP="$(mktemp -d "${TMPDIR:-/tmp}/learn-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
TMP="$(cd -P "$TMP" && pwd -P)"

export HOME="$TMP/home"
export AGENT_LEARNINGS_HOME="$TMP/store"
export XDG_CONFIG_HOME="$HOME/.config"
export GIT_CONFIG_GLOBAL="$HOME/.gitconfig"
export GIT_CONFIG_NOSYSTEM=1
unset LEARN_TODAY LEARN_PROJECT
mkdir -p "$HOME" "$TMP/proj"
git config --global user.name "learn test"
git config --global user.email "learn@example.invalid"
git config --global init.defaultBranch main

FAILS=0
PASSES=0
OUT=""
RC=0

STORE_HOME_T="$AGENT_LEARNINGS_HOME"
pass() { PASSES=$((PASSES + 1)); echo "PASS: $1"; }
fail() { FAILS=$((FAILS + 1)); echo "FAIL: $1"; [ -z "${2:-}" ] || printf '      %s\n' "$2"; }

check() {   # check <name> <command...>
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then pass "$name"; else fail "$name"; fi
}

# run <args...>: capture combined output in OUT, status in RC.
run() {
  OUT="$("$LEARN" "$@" 2>&1)"; RC=$?
}

expect_ok() {   # expect_ok <name> <args...>
  local name="$1"; shift
  run "$@"
  if [ "$RC" -eq 0 ]; then pass "$name"; else fail "$name" "exit $RC: $OUT"; fi
}

expect_die() {   # expect_die <name> <stderr-substring> <args...>
  local name="$1" sub="$2"; shift 2
  run "$@"
  if [ "$RC" -ne 0 ] && [[ "$OUT" == *"$sub"* ]]; then pass "$name"; else fail "$name" "exit $RC: $OUT"; fi
}

expect_contains() {   # expect_contains <name> <needle>   (against OUT)
  if [[ "$OUT" == *"$2"* ]]; then pass "$1"; else fail "$1" "missing '$2' in: $OUT"; fi
}

expect_not_contains() {
  if [[ "$OUT" != *"$2"* ]]; then pass "$1"; else fail "$1" "unexpected '$2' in: $OUT"; fi
}

fmval() { awk -v k="$2" 'NR==1{next} /^---$/{exit} index($0,k": ")==1{print substr($0,length(k)+3); exit}' "$1"; }

cd "$TMP/proj" || exit 1
export LEARN_PROJECT="proj-a"

STORE="$TMP/store"
PDIR="$STORE/work"

# --- basics ---------------------------------------------------------------
run --version
if [ "$RC" -eq 0 ] && [[ "$OUT" == learn\ * ]]; then pass "--version"; else fail "--version" "$OUT"; fi
expect_ok "help" help
expect_die "unknown command dies" "unknown command" nope

# --- init -----------------------------------------------------------------
expect_die "init without profile lists profiles" "usage: learn init" init
expect_ok "init work" init work
check "store is git repo" test -d "$STORE/.git"
check "profile dirs created" test -d "$PDIR/inbox" -a -d "$PDIR/candidates" -a -d "$PDIR/applied" -a -d "$PDIR/rejected"
check "INDEX.md created" test -f "$PDIR/INDEX.md"
check "markers.json created" test -f "$STORE/markers.json"
if command -v node >/dev/null 2>&1; then
  check "markers.json is valid JSON" node -e 'JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"))' "$STORE/markers.json"
fi
check ".learnings symlink -> profile" test "$(readlink .learnings)" = "$PDIR"
check "global gitignore has .learnings (no trailing slash)" grep -qxF '.learnings' "$HOME/.config/git/ignore"
( cd "$TMP/proj" && git init -q . 2>/dev/null )
check "git ignores the .learnings symlink" git -C "$TMP/proj" check-ignore -q .learnings
if git -C "$TMP/proj" status --porcelain | grep -q '\.learnings'; then fail "git status hides .learnings" "$(git -C "$TMP/proj" status --porcelain)"; else pass "git status hides .learnings"; fi
rm -rf "$TMP/proj/.git"
# legacy '.learnings/' (directory-only pattern) is rewritten in place, other lines kept
printf '%s\n' '**/.claude/settings.local.json' '.memory/' '.learnings/' > "$HOME/.config/git/ignore"
expect_ok "init repairs legacy .learnings/ pattern" init work
expect_contains "init reports the gitignore fix" "fixed global gitignore"
check "legacy pattern replaced" test "$(grep -cxF '.learnings' "$HOME/.config/git/ignore")" = 1 -a "$(grep -cxF '.learnings/' "$HOME/.config/git/ignore")" = 0
check "other gitignore lines kept" grep -qxF '.memory/' "$HOME/.config/git/ignore"
expect_ok "init is idempotent" init work
expect_die "init rejects bad profile" "invalid profile" init "Bad Name"
expect_die "init no longer accepts --name" "unknown option" init work --name foo

# --- add ------------------------------------------------------------------
expect_die "add rejects bad key" "invalid key" add --key BadKey --kind correction --summary "s" --evidence "e"
expect_die "add rejects long summary" "max 120" add --key test.long --kind correction --summary "$(printf 'x%.0s' $(seq 1 121))" --evidence "e"
expect_die "add rejects bad kind" "invalid kind" add --key test.badkind --kind nonsense --summary "s" --evidence "e"

expect_ok "add test.wrong-cwd" add --key test.wrong-cwd --kind correction \
  --summary "pytest в монорепо запускать из корня" --evidence "pytest tests/ -> ModuleNotFoundError conftest" \
  --behavior "Run pytest from the repo root."
F="$PDIR/candidates/test.wrong-cwd.md"
check "candidate file created" test -f "$F"
check "candidate seen: 1" test "$(fmval "$F" seen)" = "1"
check "candidate status: candidate" test "$(fmval "$F" status)" = "candidate"
check "candidate projects: [proj-a]" test "$(fmval "$F" projects)" = "[proj-a]"
check "candidate first-seen == last-seen" test "$(fmval "$F" first-seen)" = "$(fmval "$F" last-seen)"
check "candidate has Future behavior" grep -q '^## Future behavior$' "$F"
check "candidate has behavior text" grep -q '^Run pytest from the repo root\.$' "$F"
check "candidate has evidence line" grep -Eq '^- [0-9]{4}-[0-9]{2}-[0-9]{2} proj-a: pytest tests/' "$F"
expect_die "add duplicate key dies" "already exists" add --key test.wrong-cwd --kind correction --summary "s" --evidence "e"

expect_ok "add shell.no-sudo (second)" add --key shell.no-sudo --kind failure --summary "do not run sudo" --evidence "sudo: a password is required"
run add --key zzz.odd-area --kind workflow --summary "unknown area warns" --evidence "e"
if [ "$RC" -eq 0 ] && [[ "$OUT" == *"not in the vocabulary"* ]]; then pass "unknown area warns, does not die"; else fail "unknown area warns" "$OUT"; fi
G="$PDIR/candidates/shell.no-sudo.md"
check "behavior falls back to summary" grep -q '^do not run sudo$' "$G"

# --- bump -----------------------------------------------------------------
expect_ok "bump test.wrong-cwd (same project)" bump test.wrong-cwd --evidence "again wrong cwd"
check "bump seen -> 2" test "$(fmval "$F" seen)" = "2"
check "bump keeps single project" test "$(fmval "$F" projects)" = "[proj-a]"
expect_ok "bump with new project" bump test.wrong-cwd --evidence "user: запускай из корня" --project proj-b
check "bump seen -> 3" test "$(fmval "$F" seen)" = "3"
check "bump adds project" test "$(fmval "$F" projects)" = "[proj-a, proj-b]"
check "bump appended evidence (3 lines)" test "$(grep -c '^- [0-9]\{4\}-' "$F")" = "3"
check "evidence for new project" grep -q 'proj-b: user: запускай из корня' "$F"
expect_die "bump unknown key dies" "no candidate" bump test.nothing --evidence "x"

# --- index ----------------------------------------------------------------
expect_ok "index" index
EXPECTED="- [shell.no-sudo](candidates/shell.no-sudo.md) — failure ×1: do not run sudo
- [test.wrong-cwd](candidates/test.wrong-cwd.md) — correction ×3: pytest в монорепо запускать из корня
- [zzz.odd-area](candidates/zzz.odd-area.md) — workflow ×1: unknown area warns"
ACTUAL="$(cat "$PDIR/INDEX.md")"
if [ "$ACTUAL" = "$EXPECTED" ]; then pass "INDEX.md matches expected lines"; else fail "INDEX.md matches expected lines" "$ACTUAL"; fi

# bad frontmatter is skipped with a warning
printf 'no frontmatter here\n' > "$PDIR/candidates/git.broken.md"
run index
if [ "$RC" -eq 0 ] && [[ "$OUT" == *"skipping candidates/git.broken.md"* ]]; then pass "index skips bad frontmatter with warning"; else fail "index skips bad frontmatter" "$OUT"; fi
check "bad entry not in INDEX" bash -c '! grep -q git.broken "$1"' _ "$PDIR/INDEX.md"
rm -f "$PDIR/candidates/git.broken.md"
run index

# --- propose / apply / reject ---------------------------------------------
expect_ok "propose" propose test.wrong-cwd --owner "skill:pytest"
check "propose status: proposed" test "$(fmval "$F" status)" = "proposed"
check "propose suggested-owner" test "$(fmval "$F" suggested-owner)" = "skill:pytest"

expect_die "apply rejects bad owner" "invalid owner" apply test.wrong-cwd --owner nonsense --change-ref abc123
expect_ok "add with newline in owner" add --key tool.owner-nl --kind workflow --summary "owner flatten" --evidence "e" --owner $'skill:a\nb'
check "add owner newline flattened" test "$(fmval "$PDIR/candidates/tool.owner-nl.md" suggested-owner)" = "skill:a b"
expect_ok "apply new-skill owner (flattened)" apply tool.owner-nl --owner $'new-skill:foo\nbar' --change-ref "r1"
check "apply owner newline flattened" test "$(fmval "$PDIR/applied/tool.owner-nl.md" owner)" = "new-skill:foo bar"
expect_ok "add for config owner" add --key tool.owner-config --kind workflow --summary "config owner" --evidence "e"
expect_ok "apply config: owner" apply tool.owner-config --owner "config:~/.zshenv" --change-ref "r2"
check "apply config owner recorded" test "$(fmval "$PDIR/applied/tool.owner-config.md" owner)" = "config:~/.zshenv"
expect_ok "add for hint owner" add --key tool.owner-hint --kind workflow --summary "hint owner" --evidence "e"
expect_ok "apply hint: owner" apply tool.owner-hint --owner "hint:patch-reread" --change-ref "r3"
check "apply hint owner recorded" test "$(fmval "$PDIR/applied/tool.owner-hint.md" owner)" = "hint:patch-reread"
expect_die "apply error text lists config: and hint:" "config:<path> | hint:<id>" apply nonexistent.key --owner nonsense --change-ref x
expect_ok "apply" apply test.wrong-cwd --owner "skill:pytest" --change-ref "abc123"
A="$PDIR/applied/test.wrong-cwd.md"
check "apply moved to applied/" test -f "$A" -a ! -f "$F"
check "applied status" test "$(fmval "$A" status)" = "applied"
check "applied owner" test "$(fmval "$A" owner)" = "skill:pytest"
check "applied change-ref" test "$(fmval "$A" change-ref)" = "abc123"
check "applied applied-at" bash -c '[[ "$1" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]' _ "$(fmval "$A" applied-at)"
check "applied key left INDEX" bash -c '! grep -q test.wrong-cwd "$1"' _ "$PDIR/INDEX.md"

expect_die "bump on applied dies" "already applied" bump test.wrong-cwd --evidence "regression"
expect_contains "bump on applied prints owner" "skill:pytest"
check "applied file unchanged by failed bump" test "$(fmval "$A" seen)" = "3"

expect_die "reject requires reason" "usage" reject shell.no-sudo
expect_ok "reject" reject shell.no-sudo --reason "environment specific"
R="$PDIR/rejected/shell.no-sudo.md"
check "reject moved to rejected/" test -f "$R" -a ! -f "$G"
check "rejected status" test "$(fmval "$R" status)" = "rejected"
check "rejected reason" test "$(fmval "$R" reason)" = "environment specific"
expect_die "bump on rejected dies" "rejected" bump shell.no-sudo --evidence "x"

# --- --from drafts ----------------------------------------------------------
mkdir -p "$STORE/.drafts"
EVV='lacked packaging; project uses uv | a & b > c -> commit push revert'
cat > "$STORE/.drafts/d1.md" <<EOF
key: git.draft-one
kind: failure
summary: committed without tests: pushed to the wrong branch
evidence: $EVV
project: proj-d
behavior: Run tests before any commit; check the branch first.
owner: skill:git
EOF
expect_ok "add --from draft" add --from "$STORE/.drafts/d1.md"
D="$PDIR/candidates/git.draft-one.md"
check "add --from: file created" test -f "$D"
check "add --from: summary keeps ': ' after first" test "$(fmval "$D" description)" = "committed without tests: pushed to the wrong branch"
check "add --from: project" test "$(fmval "$D" projects)" = "[proj-d]"
check "add --from: owner" test "$(fmval "$D" suggested-owner)" = "skill:git"
check "add --from: evidence stored verbatim, one line" test "$(grep -c -F -- "- $(date +%Y-%m-%d) proj-d: $EVV" "$D")" = "1"
check "add --from: evidence line is the only one" test "$(grep -c '^- [0-9]\{4\}-' "$D")" = "1"
check "draft inside .drafts/ deleted after success" test ! -e "$STORE/.drafts/d1.md"

printf 'key: git.draft-two\nkind: workflow\nsummary: outside draft\nevidence: e2\n' > "$TMP/outside-draft.md"
expect_ok "add --from draft outside .drafts" add --from "$TMP/outside-draft.md"
check "draft outside .drafts/ kept" test -f "$TMP/outside-draft.md"

# relative path resolves against $PWD; flags override the draft; ---block tolerated
mkdir -p "$TMP/rel"
printf -- '---\nkey: git.draft-three\nkind: workflow\nsummary: from draft\nevidence: from draft evidence\nevidence: second match ignored\n---\n' > "$TMP/rel/d3.md"
( cd "$TMP/rel" && AGENT_LEARNINGS_HOME="$STORE" LEARN_PROJECT=proj-a "$LEARN" add --profile work --from d3.md --summary "from flag" --kind correction >/dev/null 2>&1 ); RC=$?
T3="$PDIR/candidates/git.draft-three.md"
if [ "$RC" -eq 0 ] && [ -f "$T3" ]; then pass "add --from relative path (leading --- tolerated)"; else fail "add --from relative path" "exit $RC"; fi
check "flag overrides draft summary" test "$(fmval "$T3" description)" = "from flag"
check "flag overrides draft kind" test "$(fmval "$T3" kind)" = "correction"
check "draft first match per field wins" grep -q 'proj-a: from draft evidence$' "$T3"

expect_die "add --from missing file dies" "cannot read draft" add --from "$TMP/nope.md"
expect_die "bump --from missing file dies" "cannot read draft" bump --from "$TMP/nope.md"
expect_die "reject --from missing file dies" "cannot read draft" reject --from "$TMP/nope.md"

printf 'key: git.draft-three\nevidence: %s\nproject: proj-e\n' "$EVV" > "$STORE/.drafts/b1.md"
expect_ok "bump --from draft (key from draft)" bump --from "$STORE/.drafts/b1.md"
check "bump --from: seen 2" test "$(fmval "$T3" seen)" = "2"
check "bump --from: project added" test "$(fmval "$T3" projects)" = "[proj-a, proj-e]"
check "bump --from: evidence verbatim" grep -qF -- "proj-e: $EVV" "$T3"
check "bump --from: draft deleted" test ! -e "$STORE/.drafts/b1.md"

printf 'key: git.draft-three\nevidence: draft ev\n' > "$STORE/.drafts/b2.md"
expect_ok "bump positional key overrides draft" bump git.draft-one --from "$STORE/.drafts/b2.md" --evidence "flag ev"
check "bump override: positional key bumped" test "$(fmval "$D" seen)" = "2"
check "bump override: flag evidence used" grep -q 'flag ev$' "$D"

printf 'key: git.draft-one\nreason: wrong: %s\n' "$EVV" > "$STORE/.drafts/r1.md"
expect_ok "reject --from draft" reject --from "$STORE/.drafts/r1.md"
check "reject --from: moved" test -f "$PDIR/rejected/git.draft-one.md" -a ! -f "$D"
check "reject --from: reason stored" test "$(fmval "$PDIR/rejected/git.draft-one.md" reason)" = "wrong: $EVV"
check "reject --from: draft deleted" test ! -e "$STORE/.drafts/r1.md"

printf 'key: git.draft-three\nreason: from draft\n' > "$STORE/.drafts/r2.md"
expect_ok "reject flag reason overrides draft" reject --from "$STORE/.drafts/r2.md" --reason "from flag"
check "reject override: reason from flag" test "$(fmval "$PDIR/rejected/git.draft-three.md" reason)" = "from flag"
rm -f "$PDIR/rejected/git.draft-three.md" "$PDIR/candidates/git.draft-two.md"
run index

# --- --date / date: (event date instead of today) ----------------------------
export LEARN_TODAY=2026-09-30
printf 'key: git.dated\nkind: failure\nsummary: dated event\nevidence: old event text\ndate: 2026-07-22\nproject: proj-a\n' > "$STORE/.drafts/dd1.md"
expect_ok "add --from with date:" add --from "$STORE/.drafts/dd1.md"
DD="$PDIR/candidates/git.dated.md"
check "add date: first-seen is the event date" test "$(fmval "$DD" first-seen)" = "2026-07-22"
check "add date: last-seen is the event date" test "$(fmval "$DD" last-seen)" = "2026-07-22"
check "add date: evidence prefixed once with event date" test "$(grep -c '^- ' "$DD")" = "1" -a "$(grep '^- ' "$DD")" = "- 2026-07-22 proj-a: old event text"
expect_ok "add --date flag overrides draft date" add --key git.dated-flag --kind failure --summary s --evidence e --date 2026-08-01
check "add --date: first/last-seen" test "$(fmval "$PDIR/candidates/git.dated-flag.md" first-seen)" = "2026-08-01" -a "$(fmval "$PDIR/candidates/git.dated-flag.md" last-seen)" = "2026-08-01"
expect_ok "add without date uses today" add --key git.undated --kind failure --summary s --evidence e
check "add without date: LEARN_TODAY" test "$(fmval "$PDIR/candidates/git.undated.md" first-seen)" = "2026-09-30"
expect_die "add rejects bad date" "invalid date" add --key git.baddate --kind failure --summary s --evidence e --date 2026-02-30
expect_die "add rejects non-date" "invalid date" add --key git.baddate --kind failure --summary s --evidence e --date yesterday

# bump: older date lowers first-seen, newer raises last-seen, in-between changes neither
expect_ok "bump --date older" bump git.dated --evidence "older" --date 2026-07-01
check "bump older: first-seen min" test "$(fmval "$DD" first-seen)" = "2026-07-01" -a "$(fmval "$DD" last-seen)" = "2026-07-22"
expect_ok "bump --date newer" bump git.dated --evidence "newer" --date 2026-08-15
check "bump newer: last-seen max" test "$(fmval "$DD" first-seen)" = "2026-07-01" -a "$(fmval "$DD" last-seen)" = "2026-08-15"
expect_ok "bump --date in between" bump git.dated --evidence "middle" --date 2026-07-10
check "bump middle: range unchanged" test "$(fmval "$DD" first-seen)" = "2026-07-01" -a "$(fmval "$DD" last-seen)" = "2026-08-15"
check "bump evidence lines use their own dates" grep -q '^- 2026-07-10 proj-a: middle$' "$DD"
printf 'key: git.dated\nevidence: from draft\ndate: 2026-09-01\n' > "$STORE/.drafts/dd2.md"
expect_ok "bump --from with date:" bump --from "$STORE/.drafts/dd2.md"
check "bump date: last-seen max" test "$(fmval "$DD" last-seen)" = "2026-09-01"
check "bump date: evidence dated" grep -q '^- 2026-09-01 proj-a: from draft$' "$DD"
expect_ok "bump without date uses today" bump git.dated --evidence "no date"
check "bump without date: last-seen today" test "$(fmval "$DD" last-seen)" = "2026-09-30"
expect_die "bump rejects bad date" "invalid date" bump git.dated --evidence x --date 2026-13-01
unset LEARN_TODAY
rm -f "$PDIR/candidates/git.dated.md" "$PDIR/candidates/git.dated-flag.md" "$PDIR/candidates/git.undated.md" "$PDIR/candidates/git.draft-one.md"
run index

# --- doctor ---------------------------------------------------------------
run index
run doctor
if [ "$RC" -eq 0 ] && [[ "$OUT" == *OK* ]]; then pass "doctor exits 0"; else fail "doctor exits 0" "exit $RC: $OUT"; fi
# doctor catches problems
sed 's/^owner: .*$//' "$A" > "$A.tmp" && mv "$A.tmp" "$A"
expect_die "doctor flags applied missing owner" "APPLIED MISSING owner" doctor
{ awk 'NR==1{print; print "owner: skill:pytest"; next} {print}' "$A"; } > "$A.tmp" && mv "$A.tmp" "$A"
run doctor
if [ "$RC" -eq 0 ]; then pass "doctor clean after owner restored"; else fail "doctor clean after owner restored" "$OUT"; fi
run doctor
if [ "$RC" -eq 0 ]; then pass "doctor accepts new-skill: owner"; else fail "doctor accepts new-skill: owner" "$OUT"; fi
run doctor
if [ "$RC" -eq 0 ]; then pass "doctor accepts config: and hint: owners"; else fail "doctor accepts config:/hint: owners" "$OUT"; fi
printf '%s\n' '{"version":1,"hints":[{"id":"x","tools":["grep"],"match":"(","text":"t"}]}' > "$STORE_HOME_T/hints.json"
expect_die "doctor flags bad personal hints.json" "BAD HINTS" doctor
printf '%s\n' '{"version":1,"hints":[{"id":"x","tools":["grep"],"match":"boom","text":"t"}]}' > "$STORE_HOME_T/hints.json"
run doctor
if [ "$RC" -eq 0 ]; then pass "doctor accepts valid personal hints.json"; else fail "doctor valid hints.json" "$OUT"; fi
rm -f "$STORE_HOME_T/hints.json"
cp "$PDIR/applied/tool.owner-nl.md" "$TMP/owner-nl.bak"
fm_tmp="$PDIR/applied/tool.owner-nl.md"
sed 's/^owner: .*$/owner: bogus:x/' "$fm_tmp" > "$fm_tmp.tmp" && mv "$fm_tmp.tmp" "$fm_tmp"
expect_die "doctor flags invalid owner" "BAD OWNER" doctor
cp "$TMP/owner-nl.bak" "$fm_tmp"
printf 'drift\n' >> "$PDIR/INDEX.md"
expect_die "doctor flags INDEX drift" "INDEX DRIFT" doctor
run index
run doctor

# --- status / grep --------------------------------------------------------
expect_ok "status" status
expect_contains "status shows profile" "work"
expect_contains "status shows git state" "git:"
expect_ok "grep finds text" grep "ModuleNotFoundError"
expect_contains "grep hit path" "applied/test.wrong-cwd.md"
expect_ok "grep no match still exits 0" grep "definitely-not-present-xyz"
expect_contains "grep no match message" "no matches"

# --- inbox / ack ----------------------------------------------------------
cat > "$PDIR/inbox/2026-09-29.jsonl" <<'EOF'
{"ts":"2026-09-29T10:00:00Z","kind":"failure","tool":"bash","head":"exit 2: pytest: ModuleNotFoundError","project":"proj-a","session":"ses_1"}
{"ts":"2026-09-29T10:05:00Z","kind":"correction","tier":"strong","marker":"^нет[,.]","snippet":"Нет, я про другое","project":"proj-a","session":"ses_1","prev_tool":"edit"}
EOF
cat > "$PDIR/inbox/2026-09-30.jsonl" <<'EOF'
{"ts":"2026-09-30T09:00:00Z","kind":"correction","tier":"weak","marker":"^почему","snippet":"Почему ci упал?","project":"proj-b","session":"ses_2","prev_tool":"bash"}
{"ts":"2026-09-30T09:30:00Z","kind":"confirmation","tier":"","marker":"^ок","snippet":"ок","project":"proj-b","session":"ses_2"}
EOF
run inbox
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "4" ]; then pass "inbox lists all 4 events before ack"; else fail "inbox lists all events" "$OUT"; fi
expect_contains "inbox compact shows failure text" "bash: exit 2: pytest"
expect_contains "inbox compact shows snippet" "Нет, я про другое"
run inbox --kind failure
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "1" ]; then pass "inbox --kind failure"; else fail "inbox --kind failure" "$OUT"; fi
run inbox --tier weak
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "1" ]; then pass "inbox --tier weak"; else fail "inbox --tier weak" "$OUT"; fi
run inbox --since 2026-09-30T00:00:00Z
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "2" ]; then pass "inbox --since"; else fail "inbox --since" "$OUT"; fi
run inbox --json
if [ "$(printf '%s\n' "$OUT" | grep -c '^{')" = "4" ]; then pass "inbox --json raw lines"; else fail "inbox --json" "$OUT"; fi
expect_not_contains "inbox --json has no ack trailer" "ack: learn ack"
run inbox
expect_contains "inbox trailer has max ts" "ack: learn ack --until 2026-09-30T09:30:00Z"
run inbox --kind failure
expect_contains "inbox trailer follows filtered max ts" "ack: learn ack --until 2026-09-29T10:00:00Z"
run inbox --limit 2
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "2" ]; then pass "inbox --limit 2"; else fail "inbox --limit 2" "$OUT"; fi
LIMIT_LAST="$(printf '%s\n' "$OUT" | grep '^2026-' | tail -n 1 | awk '{print $1}')"
expect_contains "inbox --limit trailer is max of the shown batch" "ack: learn ack --until $LIMIT_LAST"
run inbox --json --limit 1
if [ "$(printf '%s\n' "$OUT" | grep -c '^{')" = "1" ]; then pass "inbox --json --limit 1"; else fail "inbox --json --limit 1" "$OUT"; fi
expect_die "inbox --limit rejects 0" "positive integer" inbox --limit 0

expect_ok "ack --until" ack --until 2026-09-29T23:59:59Z
check ".ack file written" test "$(cat "$PDIR/inbox/.ack")" = "2026-09-29T23:59:59Z"
run inbox
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "2" ]; then pass "inbox honors .ack"; else fail "inbox honors .ack" "$OUT"; fi
expect_not_contains "acked events hidden" "Нет, я про другое"
run inbox --all
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "4" ]; then pass "inbox --all ignores .ack"; else fail "inbox --all" "$OUT"; fi
expect_ok "status counts unacked" status
expect_contains "status unacked count" "2 unacked"
expect_ok "ack now" ack
run inbox
expect_contains "inbox empty after ack" "(no events)"
expect_not_contains "no trailer when no events" "ack: learn ack"
expect_die "ack rejects bad timestamp" "invalid timestamp" ack --until yesterday

# --- prune ----------------------------------------------------------------
LEARN_TODAY=2026-01-01 "$LEARN" add --key env.stale-thing --kind workflow --summary "old one-off" --evidence "old" >/dev/null 2>&1
LEARN_TODAY=2026-09-30 "$LEARN" add --key env.fresh-thing --kind workflow --summary "fresh one-off" --evidence "new" >/dev/null 2>&1
LEARN_TODAY=2026-01-01 "$LEARN" add --key env.old-repeated --kind workflow --summary "old but repeated" --evidence "old" >/dev/null 2>&1
LEARN_TODAY=2026-01-02 "$LEARN" bump env.old-repeated --evidence "again" >/dev/null 2>&1
export LEARN_TODAY=2026-09-30
expect_ok "prune --days 90" prune --days 90
check "stale seen=1 moved to rejected" test -f "$PDIR/rejected/env.stale-thing.md"
check "stale reason: stale" test "$(fmval "$PDIR/rejected/env.stale-thing.md" reason)" = "stale"
check "stale status: rejected" test "$(fmval "$PDIR/rejected/env.stale-thing.md" status)" = "rejected"
check "fresh candidate kept" test -f "$PDIR/candidates/env.fresh-thing.md"
check "seen>1 candidate kept" test -f "$PDIR/candidates/env.old-repeated.md"
check "pruned key left INDEX" bash -c '! grep -q env.stale-thing "$1"' _ "$PDIR/INDEX.md"
# acked inbox files: filename date < ack date - days
printf '{"ts":"2026-05-01T10:00:00Z","kind":"failure","tool":"bash","head":"old","project":"p"}\n' > "$PDIR/inbox/2026-05-01.jsonl"
printf '{"ts":"2026-08-01T10:00:00Z","kind":"failure","tool":"bash","head":"mid","project":"p"}\n' > "$PDIR/inbox/2026-08-01.jsonl"
"$LEARN" ack --until 2026-09-30T00:00:00Z >/dev/null 2>&1
run prune --days 90
expect_contains "prune reports removed inbox count" "removed 1 acked inbox file(s)"
check "old acked inbox file removed" test ! -e "$PDIR/inbox/2026-05-01.jsonl"
check "recent acked inbox file kept" test -f "$PDIR/inbox/2026-08-01.jsonl"
check "newest inbox files kept" test -f "$PDIR/inbox/2026-09-29.jsonl" -a -f "$PDIR/inbox/2026-09-30.jsonl"
run prune --days 30
expect_contains "prune --days 30 removes more" "removed 1 acked inbox file(s)"
check "mid-age inbox file removed at --days 30" test ! -e "$PDIR/inbox/2026-08-01.jsonl"
# backfill files are named by arrival date (<date>-backfill.jsonl) and age the same way
printf '{"ts":"2026-01-01T10:00:00Z","ats":"2026-05-02T10:00:00Z","kind":"failure","tool":"bash","head":"bf-old","project":"p"}\n' > "$PDIR/inbox/2026-05-02-backfill.jsonl"
printf '{"ts":"2026-01-01T10:00:00Z","ats":"2026-09-29T10:00:00Z","kind":"failure","tool":"bash","head":"bf-new","project":"p"}\n' > "$PDIR/inbox/2026-09-29-backfill.jsonl"
run prune --days 30
expect_contains "prune counts an old backfill file" "removed 1 acked inbox file(s)"
check "old acked backfill file removed" test ! -e "$PDIR/inbox/2026-05-02-backfill.jsonl"
check "recent backfill file kept" test -f "$PDIR/inbox/2026-09-29-backfill.jsonl"
rm -f "$PDIR/inbox/2026-09-29-backfill.jsonl"
unset LEARN_TODAY
run doctor
if [ "$RC" -eq 0 ]; then pass "doctor clean after prune"; else fail "doctor clean after prune" "$OUT"; fi

# --- markers --------------------------------------------------------------
if command -v node >/dev/null 2>&1; then
  expect_ok "markers add" markers add --tier strong '^тест-маркер'
  check "marker persisted" grep -q 'тест-маркер' "$STORE/markers.json"
  run markers add --tier strong '^тест-маркер'
  expect_contains "markers add is idempotent" "already present"
  expect_die "markers add rejects bad tier" "usage" markers add --tier bogus 'x'
  run markers add --tier weak '('
  if grep -q 'export.*validateMarkers\|validateMarkers' "$ROOT/skills/self-improving/opencode/learn-core.js" 2>/dev/null; then
    if [ "$RC" -ne 0 ] && ! grep -q '"("' "$STORE/markers.json"; then pass "markers add rejects bad regex without persisting"; else fail "markers add rejects bad regex" "exit $RC: $OUT"; fi
  else
    # validateMarkers not exported yet: fallback is JSON-syntax only, with a warning.
    if [[ "$OUT" == *"validateMarkers unavailable"* ]]; then pass "markers add falls back with warning (no validateMarkers)"; else fail "markers add fallback warning" "exit $RC: $OUT"; fi
    node -e '
      const fs = require("fs"), f = process.argv[1];
      const j = JSON.parse(fs.readFileSync(f, "utf8"));
      j.correction.weak = j.correction.weak.filter((x) => x !== "(");
      fs.writeFileSync(f, JSON.stringify(j, null, 2) + "\n");
    ' "$STORE/markers.json"
  fi
  check "no stray markers tmp file" bash -c '! ls "$1"/markers.json.tmp.* >/dev/null 2>&1' _ "$STORE"
  if [ -f "$ROOT/skills/self-improving/opencode/markers.default.json" ]; then
    expect_ok "markers list" markers list
    expect_contains "markers list groups tiers" "correction/strong:"
    expect_contains "markers list shows personal" "тест-маркер"
  else
    echo "SKIP: markers list (markers.default.json absent)"
  fi
  if [ -f "$ROOT/skills/self-improving/opencode/learn-core.js" ]; then
    run markers test "Нет, я про другое"
    if [ "$RC" -eq 0 ] && [[ "$OUT" == strong* ]]; then pass "markers test -> strong"; else fail "markers test -> strong" "exit $RC: $OUT"; fi
    run markers test "просто нейтральная фраза без маркеров"
    if [ "$RC" -eq 0 ] && [[ "$OUT" == *"no match"* ]]; then pass "markers test -> no match"; else fail "markers test -> no match" "exit $RC: $OUT"; fi
  else
    echo "SKIP: markers test (learn-core.js absent)"
  fi
else
  echo "SKIP: markers (node not found)"
fi

# --- backup ---------------------------------------------------------------
expect_ok "backup" backup
expect_contains "backup message has profile" "learn(work):"
LAST="$(git -C "$STORE" log -1 --format=%s)"
if [[ "$LAST" == "learn(work): +"*" candidates, ~"*" bumped, →"*" applied"* ]]; then pass "backup commit message format"; else fail "backup commit message format" "$LAST"; fi
check "store clean after backup" test -z "$(git -C "$STORE" status --porcelain)"
expect_ok "backup with nothing to commit" backup
expect_contains "backup reports nothing" "nothing to back up"
run add --key build.after-backup --kind workflow --summary "post backup" --evidence "e"
expect_ok "backup second commit" backup
LAST="$(git -C "$STORE" log -1 --format=%s)"
if [[ "$LAST" == "learn(work): +1 candidates, ~0 bumped, →0 applied"* ]]; then pass "backup counts new candidate"; else fail "backup counts new candidate" "$LAST"; fi
expect_die "backup --push without remote dies" "no remote" backup --push

# --- install-opencode -----------------------------------------------------
if [ -f "$ROOT/skills/self-improving/opencode/self-improving.js" ]; then
  expect_ok "install-opencode" install-opencode
  check "plugin symlink created" test -L "$HOME/.config/opencode/plugins/self-improving.js"
  expect_ok "install-opencode idempotent" install-opencode
else
  echo "SKIP: install-opencode (self-improving.js absent)"
fi

# --- profile resolution ---------------------------------------------------
mkdir -p "$TMP/other"
( cd "$TMP/other" && "$LEARN" status >/dev/null 2>&1 ); RC=$?
if [ "$RC" -ne 0 ]; then pass "commands die without .learnings"; else fail "commands die without .learnings"; fi

# --- dream ----------------------------------------------------------------
# Fixture store (separate from the store above): profile "work" with unacked
# inbox events, two similar candidates, one ready for proposal (seen=3) and one
# stale (seen=1, old).
DSTORE="$TMP/dstore"
DPDIR="$DSTORE/work"
mkdir -p "$TMP/dcwd"
cd "$TMP/dcwd" || exit 1
export LEARN_TODAY=2026-09-30
export AGENT_LEARNINGS_HOME="$DSTORE"
mkdir -p "$DPDIR/inbox" "$DPDIR/candidates" "$DPDIR/applied" "$DPDIR/rejected"
git -C "$DSTORE" init -q

dcand() {   # dcand <key> <desc> <seen> <last-seen> <projects> [status]
  cat > "$DPDIR/candidates/$1.md" <<EOF
---
description: $2
pattern-key: $1
kind: correction
seen: $3
first-seen: 2025-01-01
last-seen: $4
projects: [$5]
status: ${6:-candidate}
---
## Future behavior
Do the right thing.

## Evidence
- $4 proj-a: something happened
EOF
}
dcand test.pytest-wrong-cwd "run pytest from the repo root in a monorepo" 1 2026-09-29 proj-a
dcand test.pytest-cwd-root "pytest monorepo: repo root cwd" 1 2026-09-29 proj-a
dcand git.commit-scope "commit messages need a scope" 3 2026-09-29 proj-a
dcand env.old-thing "some very old one-off" 1 2025-01-01 proj-a
cat > "$DPDIR/inbox/2026-09-29.jsonl" <<'EOF'
{"ts":"2026-09-29T10:00:00Z","kind":"failure","tool":"bash","head":"SECRET-EVENT-TEXT","project":"proj-a","session":"s1"}
{"ts":"2026-09-29T11:00:00Z","kind":"correction","tier":"strong","marker":"m","snippet":"SECRET-EVENT-TEXT-2","project":"proj-a","session":"s1"}
EOF
"$LEARN" index --profile work >/dev/null 2>&1

run dream --report
expect_contains "dream --report: unacked section" "unacked inbox"
expect_contains "dream --report: unacked count and range" "[work] 2 event(s), oldest 2026-09-29T10:00:00Z, newest 2026-09-29T11:00:00Z"
expect_not_contains "dream --report: no event text" "SECRET-EVENT-TEXT"
expect_contains "dream --report: similar section" "similar candidates"
expect_contains "dream --report: similar pair" "[work] candidates/test.pytest-cwd-root.md <-> candidates/test.pytest-wrong-cwd.md"
expect_contains "dream --report: ready section" "ready for proposal"
expect_contains "dream --report: ready item" "[work] git.commit-scope — seen 3"
expect_contains "dream --report: stale section" "stale (seen 1, older than 90 days"
expect_contains "dream --report: stale item" "[work] env.old-thing"
expect_contains "dream --report: summary" "dream: 2 unacked, 1 similar, 1 ready, 1 stale, 0 unusable"

# proposal section removes it from "ready"; ack clears "unacked"
printf '\n## Proposal\n- **Owner:** script:x\n' >> "$DPDIR/candidates/git.commit-scope.md"
"$LEARN" ack --profile work --until 2026-09-29T11:00:00Z >/dev/null 2>&1
run dream --report --profile work
expect_not_contains "dream --report: candidate with Proposal not ready" "git.commit-scope — seen"
expect_contains "dream --report: acked inbox gone from summary" "dream: 0 unacked, 1 similar, 0 ready, 1 stale, 0 unusable"

# proposed without decision (>14d), unusable and hygiene sections
dcand build.proposed-long "waiting for the user" 3 2026-09-01 proj-a proposed
printf 'no frontmatter\n' > "$DPDIR/candidates/git.broken.md"
dcand zzz.odd-area "unknown area here" 1 2026-09-29 proj-a
"$LEARN" index --profile work >/dev/null 2>&1
run dream --report
expect_contains "dream --report: proposed-without-decision section" "proposed without decision"
expect_contains "dream --report: proposed item" "[work] build.proposed-long"
expect_contains "dream --report: unusable section" "unusable"
expect_contains "dream --report: unusable frontmatter" "[work] candidates/git.broken.md"
expect_contains "dream --report: hygiene unknown area" "unknown area 'zzz'"
run dream --report --profile nope
if [ "$RC" -ne 0 ] && [[ "$OUT" == *"no such profile"* ]]; then pass "dream --report unknown --profile dies"; else fail "dream --report unknown --profile" "$OUT"; fi
expect_die "dream --report rejects --model" "usage: learn dream" dream --report --model a/b
expect_die "dream rejects --stale-days without --report" "usage: learn dream" dream --stale-days 5
run dream --report --stale-days 5000
expect_not_contains "dream --report --stale-days widens window" "[work] env.old-thing"

# --profile on ordinary commands (used by the dream agent from the store root)
run inbox --profile work --all
expect_contains "inbox --profile reads that profile" "SECRET-EVENT-TEXT"
run doctor --profile work
if [ "$RC" -ne 0 ] && [[ "$OUT" == *"BAD FRONTMATTER"* ]]; then pass "doctor --profile flags bad frontmatter"; else fail "doctor --profile" "exit $RC: $OUT"; fi
rm -f "$DPDIR/candidates/git.broken.md" "$DPDIR/candidates/zzz.odd-area.md" "$DPDIR/candidates/build.proposed-long.md"
"$LEARN" index --profile work >/dev/null 2>&1
run doctor --profile work
if [ "$RC" -eq 0 ]; then pass "doctor --profile clean"; else fail "doctor --profile clean" "$OUT"; fi

# tidy store
TSTORE="$TMP/tstore"
mkdir -p "$TSTORE/home/inbox" "$TSTORE/home/candidates" "$TSTORE/home/applied" "$TSTORE/home/rejected"
git -C "$TSTORE" init -q
AGENT_LEARNINGS_HOME="$TSTORE" "$LEARN" add --profile home --key tool.only-one --kind workflow --summary "the only candidate" --evidence "e" --project proj-a >/dev/null 2>&1
AGENT_LEARNINGS_HOME="$TSTORE" "$LEARN" ack --profile home --until 2026-09-30T00:00:00Z >/dev/null 2>&1
OUT="$(AGENT_LEARNINGS_HOME="$TSTORE" "$LEARN" dream --report 2>&1)"; RC=$?
expect_contains "dream --report tidy store" "dream: learnings are tidy — nothing to consolidate"

# --print-config: placeholders substituted, valid JSON
run dream --print-config
if [ "$RC" -eq 0 ]; then pass "dream --print-config exits 0"; else fail "dream --print-config exits 0" "$OUT"; fi
expect_not_contains "print-config: no __STORE__" "__STORE__"
expect_not_contains "print-config: no __LEARN_BIN__" "__LEARN_BIN__"
expect_not_contains "print-config: no __HOME__" "__HOME__"
expect_contains "print-config: store substituted" "edit:$DSTORE/*"
expect_contains "print-config: learn bin substituted" "shell:$LEARN inbox*"
expect_contains "print-config: home substituted" "read:$HOME/.ssh/*"
expect_contains "print-config: learn-dream agent" '"learn-dream"'
if command -v node >/dev/null 2>&1; then
  printf '%s\n' "$OUT" > "$TMP/dream-config.json"
  check "print-config is valid JSON" node -e 'JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"))' "$TMP/dream-config.json"
fi

# dream without opencode on PATH
NOOC_PATH="$TMP/emptybin:/usr/bin:/bin"
mkdir -p "$TMP/emptybin"
if PATH="$NOOC_PATH" command -v opencode >/dev/null 2>&1; then
  echo "SKIP: dream without opencode (opencode exists in /usr/bin or /bin)"
else
  OUT="$(PATH="$NOOC_PATH" "$LEARN" dream 2>&1)"; RC=$?
  if [ "$RC" -ne 0 ] && [[ "$OUT" == *"opencode not found on PATH"* ]]; then pass "dream without opencode dies cleanly"; else fail "dream without opencode dies cleanly" "exit $RC: $OUT"; fi
fi

# dream with a stub opencode: pre/post commits, .dream-last, undo hint
STUBBIN="$TMP/stubbin"
mkdir -p "$STUBBIN"
cat > "$STUBBIN/opencode" <<'EOF'
#!/usr/bin/env bash
# stub: record the invocation, touch a candidate, print a summary line.
{
  printf 'cwd=%s\n' "$(pwd -P)"
  printf 'home=%s\n' "$AGENT_LEARNINGS_HOME"
  printf 'args=%s\n' "$*"
  printf 'config=%s\n' "$OPENCODE_CONFIG_CONTENT"
  printf 'db=%s\n' "$OPENCODE_DB"
  if [ -e "$OPENCODE_DB" ] || [ -e "$OPENCODE_DB-wal" ] || [ -e "$OPENCODE_DB-shm" ]; then
    echo "db-existed=yes"
  else
    echo "db-existed=no"
  fi
} > "$STUB_LOG"
printf '\nstub touched\n' >> "$AGENT_LEARNINGS_HOME/work/candidates/git.commit-scope.md"
mkdir -p "$AGENT_LEARNINGS_HOME/.drafts"
echo "leftover" > "$AGENT_LEARNINGS_HOME/.drafts/draft.md"
# STUB_INBOX=1: the plugin appends an inbox event while the dream runs.
if [ "${STUB_INBOX:-0}" = 1 ]; then
  mkdir -p "$AGENT_LEARNINGS_HOME/work/inbox"
  printf '{"ts":"2026-09-30T23:00:00Z","kind":"failure","tool":"bash","head":"during-dream","project":"proj-a","session":"s9"}\n' \
    >> "$AGENT_LEARNINGS_HOME/work/inbox/$(date +%Y-%m-%d).jsonl"
fi
# STUB_BACKUP=1: the plugin's end-of-turn `learn backup` fires mid-dream.
if [ "${STUB_BACKUP:-0}" = 1 ]; then
  {
    echo "head_before=$(git -C "$AGENT_LEARNINGS_HOME" rev-parse HEAD)"
    (cd "$STUB_BACKUP_CWD" && "$LEARN_FOR_STUB" backup 2>&1)
    echo "head_after=$(git -C "$AGENT_LEARNINGS_HOME" rev-parse HEAD)"
  } > "$STUB_BACKUP_LOG" 2>&1
fi
[ "${STUB_FAIL:-0}" = 1 ] && { echo "boom" >&2; exit 3; }
echo "working…"
echo "triaged 2 events → +1 candidates, ~0 bumped, 1 noise / 0 merged / 0 proposed"
EOF
chmod +x "$STUBBIN/opencode"
export STUB_LOG="$TMP/stub.log"
"$LEARN" index --profile work >/dev/null 2>&1   # store is dirty (uncommitted fixture) -> pre-dream snapshot
git -C "$DSTORE" config user.name t; git -C "$DSTORE" config user.email t@example.invalid
PRE_COMMITS="$(git -C "$DSTORE" rev-list --count HEAD 2>/dev/null || echo 0)"
# --report / --print-config must not touch an existing dream DB
echo stale > "$DSTORE/.git/learn-dream.db"
echo stale > "$DSTORE/.git/learn-dream.db-wal"
run dream --report
check "dream --report keeps .git/learn-dream.db" test -e "$DSTORE/.git/learn-dream.db"
run dream --print-config
check "dream --print-config keeps .git/learn-dream.db" test -e "$DSTORE/.git/learn-dream.db"
OUT="$(PATH="$STUBBIN:$PATH" "$LEARN" dream --model prov/model-x 2>&1)"; RC=$?
if [ "$RC" -eq 0 ]; then pass "dream with stub opencode exits 0"; else fail "dream with stub opencode" "exit $RC: $OUT"; fi
check "stub got OPENCODE_DB in store .git" grep -qxF "db=$DSTORE/.git/learn-dream.db" "$STUB_LOG"
check "stale dream db removed before the run" grep -qxF "db-existed=no" "$STUB_LOG"
expect_contains "dream prints session transcript location" "session transcript in"
check "dream db never shows in git status" test -z "$(git -C "$DSTORE" status --porcelain --untracked-files=all | grep 'learn-dream\.db')"
expect_contains "dream prints undo hint" "undo: git -C $DSTORE revert HEAD"
SUBJECTS="$(git -C "$DSTORE" log --format=%s)"
if [[ "$SUBJECTS" == *"learn: pre-dream snapshot"* ]]; then pass "dream made pre-dream snapshot commit"; else fail "pre-dream snapshot" "$SUBJECTS"; fi
if [ "$(git -C "$DSTORE" log -1 --format=%s)" = "learn: dream — triaged 2 events → +1 candidates, ~0 bumped, 1 noise / 0 merged / 0 proposed" ]; then pass "dream commit message uses agent summary"; else fail "dream commit message" "$(git -C "$DSTORE" log -1 --format=%s)"; fi
check "dream leaves store clean" test -z "$(git -C "$DSTORE" status --porcelain --untracked-files=all)"
check "dream: .drafts/ not in git status" test -z "$(git -C "$DSTORE" status --porcelain --untracked-files=all | grep '\.drafts')"
check "dream: leftover draft removed" test ! -e "$DSTORE/.drafts/draft.md"
check "dream: .drafts/ not committed" test -z "$(git -C "$DSTORE" ls-files .drafts)"
check "store exclude lists .drafts/" grep -qxF '/.drafts/' "$DSTORE/.git/info/exclude"
check ".dream-last == HEAD" test "$(cat "$DSTORE/.dream-last")" = "$(git -C "$DSTORE" rev-parse HEAD)"
check "stub ran in store root" grep -q "^cwd=$DSTORE$" "$STUB_LOG"
check "stub got pinned AGENT_LEARNINGS_HOME" grep -q "^home=$DSTORE$" "$STUB_LOG"
check "stub got --standalone --agent learn-dream --model" grep -q '^args=run --standalone --agent learn-dream --model prov/model-x ' "$STUB_LOG"
check "stub config has no placeholders" bash -c '! grep -q "__STORE__\|__LEARN_BIN__\|__HOME__" "$1"' _ "$STUB_LOG"
check "prompt reached the stub" grep -q "Goal: consolidate the store" "$STUB_LOG"

# --if-changed: skip when nothing changed since .dream-last
HEAD_BEFORE="$(git -C "$DSTORE" rev-parse HEAD)"
rm -f "$STUB_LOG"
OUT="$(PATH="$STUBBIN:$PATH" "$LEARN" dream --if-changed 2>&1)"; RC=$?
if [ "$RC" -eq 0 ] && [[ "$OUT" == *"skipping"* ]] && [ ! -e "$STUB_LOG" ]; then pass "dream --if-changed skips when .dream-last == HEAD"; else fail "dream --if-changed skip" "exit $RC: $OUT"; fi
check "skip made no commit" test "$(git -C "$DSTORE" rev-parse HEAD)" = "$HEAD_BEFORE"
# a change outside candidates/inbox/applied/rejected does not trigger a run
echo x > "$DSTORE/notes.txt"
OUT="$(PATH="$STUBBIN:$PATH" "$LEARN" dream --if-changed 2>&1)"; RC=$?
if [ "$RC" -eq 0 ] && [ ! -e "$STUB_LOG" ]; then pass "dream --if-changed ignores unrelated files"; else fail "dream --if-changed unrelated" "exit $RC: $OUT"; fi
# a new inbox event triggers a run
printf '{"ts":"2026-09-30T08:00:00Z","kind":"failure","tool":"bash","head":"x","project":"proj-a","session":"s2"}\n' > "$DPDIR/inbox/2026-09-30.jsonl"
OUT="$(PATH="$STUBBIN:$PATH" "$LEARN" dream --if-changed 2>&1)"; RC=$?
if [ "$RC" -eq 0 ] && [ -e "$STUB_LOG" ]; then pass "dream --if-changed runs after inbox change"; else fail "dream --if-changed runs" "exit $RC: $OUT"; fi
check ".dream-last advanced to HEAD" test "$(cat "$DSTORE/.dream-last")" = "$(git -C "$DSTORE" rev-parse HEAD)"

# failing opencode: die, leave changes uncommitted, keep .dream-last
LAST_SHA="$(cat "$DSTORE/.dream-last")"
OUT="$(STUB_FAIL=1 PATH="$STUBBIN:$PATH" "$LEARN" dream 2>&1)"; RC=$?
if [ "$RC" -ne 0 ] && [[ "$OUT" == *"left uncommitted for review"* ]]; then pass "dream dies when opencode fails"; else fail "dream opencode failure" "exit $RC: $OUT"; fi
check "failed dream keeps changes uncommitted" test -n "$(git -C "$DSTORE" status --porcelain --untracked-files=all)"
check "failed dream keeps .dream-last" test "$(cat "$DSTORE/.dream-last")" = "$LAST_SHA"
unset STUB_LOG LEARN_TODAY
cd "$TMP/proj" || exit 1

# dream creates .drafts/ before the agent starts (its write tool may not create parent dirs)
rm -rf "$DSTORE/.drafts"
OUT="$(STUB_FAIL=1 PATH="$STUBBIN:$PATH" AGENT_LEARNINGS_HOME="$DSTORE" "$LEARN" dream 2>&1)"
check "dream pre-creates .drafts/ before the agent runs" test -d "$DSTORE/.drafts"
check "failed dream (STUB_FAIL) leaves no .dream.lock" test ! -e "$DSTORE/.dream.lock"

# --- dream lock -------------------------------------------------------------
export STUB_LOG="$TMP/stub.log"
export LEARN_FOR_STUB="$LEARN"
export AGENT_LEARNINGS_HOME="$DSTORE"
LOCK="$DSTORE/.dream.lock"
check "store exclude lists .dream.lock" grep -qxF '/.dream.lock' "$DSTORE/.git/info/exclude"
# the store is dirty here (failed dreams leave changes) -> a backup would commit
sleep 30 & LIVE_PID=$!
echo "$LIVE_PID" > "$LOCK"
HEAD_LOCKED="$(git -C "$DSTORE" rev-parse HEAD)"
check "store is dirty before the live-lock checks" test -n "$(git -C "$DSTORE" status --porcelain --untracked-files=all)"
run backup
expect_contains "backup with live dream lock is skipped" "backup skipped"
expect_contains "backup skip names the dream pid" "pid $LIVE_PID"
if [ "$RC" -eq 0 ]; then pass "backup skip exits 0"; else fail "backup skip exits 0" "exit $RC: $OUT"; fi
check "backup with live lock makes no commit" test "$(git -C "$DSTORE" rev-parse HEAD)" = "$HEAD_LOCKED"
check "backup with live lock stages nothing" test -z "$(git -C "$DSTORE" diff --cached --name-only)"
run status --profile work
expect_contains "status shows running dream" "running (pid $LIVE_PID)"
rm -f "$STUB_LOG"
OUT="$(PATH="$STUBBIN:$PATH" "$LEARN" dream 2>&1)"; RC=$?
if [ "$RC" -ne 0 ] && [[ "$OUT" == *"another learn dream is running (pid $LIVE_PID)"* ]]; then pass "dream dies while another dream holds the lock"; else fail "dream with live lock" "exit $RC: $OUT"; fi
check "refused dream did not run the agent" test ! -e "$STUB_LOG"
check "refused dream leaves the other lock alone" test "$(cat "$LOCK")" = "$LIVE_PID"
kill "$LIVE_PID" 2>/dev/null; wait "$LIVE_PID" 2>/dev/null

# stale lock (dead pid): backup proceeds, dream replaces it and cleans up
true & DEAD_PID=$!; wait "$DEAD_PID" 2>/dev/null
echo "$DEAD_PID" > "$LOCK"
run status --profile work
expect_not_contains "status ignores a stale lock" "running (pid"
run backup
expect_contains "backup with stale lock commits" "backed up:"
check "backup with stale lock moved HEAD" test "$(git -C "$DSTORE" rev-parse HEAD)" != "$HEAD_LOCKED"
echo "$DEAD_PID" > "$LOCK"
OUT="$(PATH="$STUBBIN:$PATH" "$LEARN" dream 2>&1)"; RC=$?
if [ "$RC" -eq 0 ] && [ -e "$STUB_LOG" ]; then pass "dream replaces a stale lock"; else fail "dream with stale lock" "exit $RC: $OUT"; fi
check "no .dream.lock after successful dream" test ! -e "$LOCK"
echo "not a pid" > "$LOCK"
OUT="$(PATH="$STUBBIN:$PATH" "$LEARN" dream 2>&1)"; RC=$?
if [ "$RC" -eq 0 ]; then pass "dream replaces an unreadable lock"; else fail "dream with garbage lock" "exit $RC: $OUT"; fi
check "no .dream.lock after dream over garbage lock" test ! -e "$LOCK"
OUT="$(STUB_FAIL=1 PATH="$STUBBIN:$PATH" "$LEARN" dream 2>&1)"; RC=$?
if [ "$RC" -ne 0 ]; then pass "STUB_FAIL dream still fails"; else fail "STUB_FAIL dream exits non-zero" "$OUT"; fi
check "no .dream.lock after STUB_FAIL dream" test ! -e "$LOCK"
# --if-changed skip path releases the lock too
"$LEARN" backup >/dev/null 2>&1
PATH="$STUBBIN:$PATH" "$LEARN" dream >/dev/null 2>&1
OUT="$(PATH="$STUBBIN:$PATH" "$LEARN" dream --if-changed 2>&1)"; RC=$?
if [[ "$OUT" == *"skipping"* ]]; then pass "dream --if-changed skips (lock test)"; else fail "dream --if-changed skip (lock test)" "exit $RC: $OUT"; fi
check "no .dream.lock after --if-changed skip" test ! -e "$LOCK"
# the lock is local bookkeeping: never tracked, never committed
check ".dream.lock not in git ls-files" test -z "$(git -C "$DSTORE" ls-files | grep 'dream.lock')"
check ".dream.lock in no commit" test -z "$(git -C "$DSTORE" log --all --name-only --format= | grep 'dream.lock')"
check "store clean after lock scenarios" test -z "$(git -C "$DSTORE" status --porcelain --untracked-files=all)"

# inbox events recorded during the dream get their own commit before the dream commit
OUT="$(STUB_INBOX=1 PATH="$STUBBIN:$PATH" "$LEARN" dream 2>&1)"; RC=$?
if [ "$RC" -eq 0 ]; then pass "dream with STUB_INBOX exits 0"; else fail "dream with STUB_INBOX" "exit $RC: $OUT"; fi
SUBJ2="$(git -C "$DSTORE" log --format=%s -2)"
EXPECT_SUBJ2="$(printf '%s\n%s' "learn: dream — triaged 2 events → +1 candidates, ~0 bumped, 1 noise / 0 merged / 0 proposed" "learn: inbox events recorded during dream")"
if [ "$SUBJ2" = "$EXPECT_SUBJ2" ]; then pass "dream commit follows a separate inbox-events commit"; else fail "inbox-events commit order" "$SUBJ2"; fi
check "inbox-events commit holds the jsonl" test -n "$(git -C "$DSTORE" show --name-only --format= HEAD~1 | grep 'inbox/.*\.jsonl')"
check "dream commit holds no inbox jsonl" test -z "$(git -C "$DSTORE" show --name-only --format= HEAD | grep 'inbox/.*\.jsonl')"
check "dream commit holds the agent's changes" test -n "$(git -C "$DSTORE" show --name-only --format= HEAD | grep 'candidates/')"
check "store clean after STUB_INBOX dream" test -z "$(git -C "$DSTORE" status --porcelain --untracked-files=all)"
check "no .dream.lock after STUB_INBOX dream" test ! -e "$LOCK"

# a backup fired mid-dream (the plugin's end-of-turn hook) is skipped
mkdir -p "$TMP/bproj"
ln -sfn "$DPDIR" "$TMP/bproj/.learnings"
export STUB_BACKUP_CWD="$TMP/bproj" STUB_BACKUP_LOG="$TMP/stub-backup.log"
rm -f "$STUB_BACKUP_LOG"
OUT="$(STUB_BACKUP=1 PATH="$STUBBIN:$PATH" "$LEARN" dream 2>&1)"; RC=$?
if [ "$RC" -eq 0 ]; then pass "dream with mid-dream backup exits 0"; else fail "dream with STUB_BACKUP" "exit $RC: $OUT"; fi
check "mid-dream backup was skipped" grep -q 'backup skipped' "$STUB_BACKUP_LOG"
check "mid-dream backup made no commit" test "$(grep '^head_before=' "$STUB_BACKUP_LOG" | cut -d= -f2)" = "$(grep '^head_after=' "$STUB_BACKUP_LOG" | cut -d= -f2)"
check "dream commit is HEAD after mid-dream backup" test "$(git -C "$DSTORE" log -1 --format=%s | cut -c1-13)" = "learn: dream "
check "dream commit holds the agent's changes (mid-dream backup)" test -n "$(git -C "$DSTORE" show --name-only --format= HEAD | grep 'candidates/')"
check "no .dream.lock after mid-dream backup dream" test ! -e "$LOCK"
unset STUB_LOG STUB_BACKUP_CWD STUB_BACKUP_LOG LEARN_FOR_STUB

# --- inbox --limit: ties and line order ---------------------------------------
TIES="$TMP/ties"
mkdir -p "$TIES/work/inbox" "$TIES/work/candidates" "$TIES/work/applied" "$TIES/work/rejected"
git -C "$TIES" init -q
tev() { printf '{"ts":"2026-09-30T%s","kind":"failure","tool":"bash","head":"%s","project":"proj-a"}\n' "$1" "$2"; }
{ tev 10:00:01Z a1; tev 10:00:02Z b1; tev 10:00:02Z b2; tev 10:00:03Z c1; tev 10:00:00Z late; } > "$TIES/work/inbox/2026-09-30.jsonl"
tl() { AGENT_LEARNINGS_HOME="$TIES" "$LEARN" "$@"; }
OUT="$(tl inbox --profile work --limit 2 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "2" ]; then pass "inbox --limit 2: two events"; else fail "inbox --limit 2" "$OUT"; fi
if [ "$(printf '%s\n' "$OUT" | grep '^2026-' | awk '{print $1}' | paste -sd' ' -)" = "2026-09-30T10:00:00Z 2026-09-30T10:00:01Z" ]; then pass "inbox --limit 2: sorted by ts, late-appended line first"; else fail "inbox --limit 2 order" "$OUT"; fi
expect_contains "inbox --limit 2: trailer is 10:00:01" "ack: learn ack --until 2026-09-30T10:00:01Z"
OUT="$(tl inbox --profile work --limit 3 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "4" ]; then pass "inbox --limit 3 extends the tie (4 events)"; else fail "inbox --limit 3 tie" "$OUT"; fi
expect_contains "inbox --limit 3: trailer is 10:00:02" "ack: learn ack --until 2026-09-30T10:00:02Z"
expect_not_contains "inbox --limit 3: next ts not shown" "10:00:03Z  "
OUT="$(tl inbox --profile work --json --limit 3 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^{')" = "4" ]; then pass "inbox --json --limit 3 extends the tie (4 lines)"; else fail "inbox --json --limit 3 tie" "$OUT"; fi
if [[ "$(printf '%s\n' "$OUT" | head -n 1)" == *'"ts":"2026-09-30T10:00:00Z"'* ]]; then pass "inbox --json --limit 3: first line is 10:00:00"; else fail "inbox --json order" "$OUT"; fi
OUT="$(tl inbox --profile work --json 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^{')" = "5" ]; then pass "inbox --json without limit keeps all 5"; else fail "inbox --json all" "$OUT"; fi
tl ack --profile work --until 2026-09-30T10:00:02Z >/dev/null 2>&1
OUT="$(tl inbox --profile work 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "1" ] && [[ "$OUT" == *"2026-09-30T10:00:03Z"* ]]; then pass "after ack --until 10:00:02 only 10:00:03 remains"; else fail "ack after tie batch" "$OUT"; fi

# spaced JSON lines (e.g. from python json.dumps) parse like compact ones
SSTORE="$TMP/sstore"
mkdir -p "$SSTORE/work/inbox" "$SSTORE/work/candidates" "$SSTORE/work/applied" "$SSTORE/work/rejected"
git -C "$SSTORE" init -q
printf '%s\n' '{"ts": "2026-07-22T10:00:00Z", "kind": "failure", "tool": "bash", "head": "exit 1: boom", "project": "proj-a"}' > "$SSTORE/work/inbox/2026-07-22.jsonl"
OUT="$(AGENT_LEARNINGS_HOME="$SSTORE" "$LEARN" inbox --profile work 2>&1)"
expect_contains "inbox parses spaced JSON" "2026-07-22T10:00:00Z  failure  proj-a  bash: exit 1: boom"
expect_contains "inbox trailer from spaced JSON" "ack: learn ack --until 2026-07-22T10:00:00Z"
OUT="$(AGENT_LEARNINGS_HOME="$SSTORE" "$LEARN" dream --report 2>&1)"
expect_contains "dream --report oldest/newest from spaced JSON" "oldest 2026-07-22T10:00:00Z, newest 2026-07-22T10:00:00Z"

# --- ats: arrival key ------------------------------------------------------------
ASTORE="$TMP/astore"
mkdir -p "$ASTORE/work/inbox" "$ASTORE/work/candidates" "$ASTORE/work/applied" "$ASTORE/work/rejected"
git -C "$ASTORE" init -q
aev() { printf '{"ts":"%s","kind":"failure","tool":"bash","head":"%s","project":"proj-a"}\n' "$1" "$2"; }
aev2() { printf '{"ts":"%s","ats":"%s","kind":"failure","tool":"bash","head":"%s","project":"proj-a"}\n' "$1" "$2" "$3"; }
{ aev 2026-09-30T10:00:00Z plain1; aev2 2026-01-05T08:00:00Z 2026-09-30T12:00:00Z backfilled; aev 2026-09-30T11:00:00Z plain2; } > "$ASTORE/work/inbox/2026-09-30.jsonl"
al() { AGENT_LEARNINGS_HOME="$ASTORE" "$LEARN" "$@"; }
al ack --profile work --until 2026-09-30T10:30:00Z >/dev/null 2>&1
OUT="$(al inbox --profile work 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "2" ]; then pass "ats: old-ts event with new ats shown after ack"; else fail "ats shown after ack" "$OUT"; fi
if [ "$(printf '%s\n' "$OUT" | grep '^2026-' | awk '{print $1}' | paste -sd' ' -)" = "2026-09-30T11:00:00Z 2026-01-05T08:00:00Z" ]; then pass "ats: ordered by ats, ts column still shown"; else fail "ats order" "$OUT"; fi
expect_contains "ats: trailer prints max ats" "ack: learn ack --until 2026-09-30T12:00:00Z"
OUT="$(al inbox --profile work --json 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^{')" = "2" ] && [[ "$OUT" == *'"ats":"2026-09-30T12:00:00Z"'* ]]; then pass "ats: --json prints raw lines"; else fail "ats --json" "$OUT"; fi
OUT="$(al inbox --profile work --since 2026-06-01T00:00:00Z 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "1" ]; then pass "ats: --since filters on ts"; else fail "ats --since" "$OUT"; fi
OUT="$(al dream --report 2>&1)"
expect_contains "ats: dream --report counts by key, oldest/newest by ts" "2 event(s), oldest 2026-01-05T08:00:00Z, newest 2026-09-30T11:00:00Z"
al ack --profile work --until 2026-09-30T12:00:00Z >/dev/null 2>&1
OUT="$(al inbox --profile work 2>&1)"
expect_contains "ats: acking up to max ats hides the event" "(no events)"
# --limit tie extension on ats
{ aev2 2026-01-01T00:00:00Z 2026-10-01T00:00:01Z t1; aev2 2026-01-02T00:00:00Z 2026-10-01T00:00:02Z t2; aev2 2026-01-03T00:00:00Z 2026-10-01T00:00:02Z t3; aev 2026-10-01T00:00:03Z t4; } > "$ASTORE/work/inbox/2026-10-01-backfill.jsonl"
OUT="$(al inbox --profile work --limit 2 2>&1)"
if [ "$(printf '%s\n' "$OUT" | grep -c '^2026-')" = "3" ]; then pass "ats: --limit 2 extends tie on ats (3 events)"; else fail "ats --limit tie" "$OUT"; fi
expect_contains "ats: --limit trailer is max ats of batch" "ack: learn ack --until 2026-10-01T00:00:02Z"
expect_not_contains "ats: --limit excludes next key" "t4"

echo
echo "passed: $PASSES, failed: $FAILS"
[ "$FAILS" -eq 0 ]
