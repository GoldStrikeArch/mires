#!/usr/bin/env bash
# checkrun.sh: scripts/check.sh in the worktree, twice.
#   pristine  as committed: must exit 0, must run the chunk suite, must not build a sysroot.
#   broken    with one line appended to 265_chunk_static_entries.expected (restored afterwards):
#             the chunk suite's builds of 265 no longer print the expectation, so check.sh must
#             exit non-zero and name them.
# Logs in logs/check-{pristine,broken}.log; a summary in gates/check.txt.
set -uo pipefail
source "$(dirname "$0")/env.sh"
G=$V/gates
mkdir -p "$G"
cd "$W" || exit 2
before=$(ls build/sysroots | sort | tr '\n' ' ')
bash scripts/check.sh > "$V/logs/check-pristine.log" 2>&1
pristine=$?
after=$(ls build/sysroots | sort | tr '\n' ' ')
exp=examples/core-tests/265_chunk_static_entries.expected
cp "$exp" "$V/logs/265.expected.saved"
echo "999" >> "$exp"
bash scripts/check.sh > "$V/logs/check-broken.log" 2>&1
broken=$?
cp "$V/logs/265.expected.saved" "$exp"
{
  echo "pristine: exit $pristine; chunk suite line: $(grep -E '^chunk suite:' "$V/logs/check-pristine.log"); sysroot built by chunk-test: $(grep -c 'building matching keyed sysroot' "$V/logs/check-pristine.log"); sysroot directories unchanged: $([ "$before" = "$after" ] && echo yes || echo no)"
  echo "broken:   exit $broken; chunk suite line: $(grep -E '^chunk suite:' "$V/logs/check-broken.log"); $(grep -E '^FAIL' "$V/logs/check-broken.log" | cut -c1-120 | tr '\n' ';')"
  echo "worktree status after restore: $(git status --short | tr '\n' ';')"
} > "$G/check.txt"
cat "$G/check.txt"
