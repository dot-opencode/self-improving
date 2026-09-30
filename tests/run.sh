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
check "global gitignore has .learnings/" grep -qxF '.learnings/' "$HOME/.config/git/ignore"
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

echo
echo "passed: $PASSES, failed: $FAILS"
[ "$FAILS" -eq 0 ]
