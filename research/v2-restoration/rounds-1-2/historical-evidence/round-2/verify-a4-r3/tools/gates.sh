#!/usr/bin/env bash
# gates.sh: every COMMON.md gate from the verifier worktree root, one log each, plus summary.tsv.
set -uo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-a3724c72be1271b05
G=$V/gates
mkdir -p "$G"
: > "$G/summary.tsv"
cd "$W"
gate() {
  local name=$1; shift
  ( "$@" ) > "$G/$name.log" 2>&1
  local code=$?
  printf '%s\t%s\n' "$name" "$code" >> "$G/summary.tsv"
}
gate check bash scripts/check.sh
gate fixtures env SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on' bash scripts/test.sh --require-core
gate active-poll env SKIP_BUILD=1 bash scripts/active-poll-cleanup-test.sh
gate extern env SKIP_BUILD=1 bash scripts/extern-test.sh
gate standalone node scripts/standalone-check.mjs
gate known-gaps python3 scripts/check-known-gaps.py
gate round1-open python3 scripts/check-round1-open.py
cp build/round1-open/receipt.json "$G/round1-open.receipt.json" 2>/dev/null
gate round1-open-oz-inline env JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on python3 scripts/check-round1-open.py
cp build/round1-open/receipt.json "$G/round1-open-oz-inline.receipt.json" 2>/dev/null
echo gates-done >> "$G/summary.tsv"
