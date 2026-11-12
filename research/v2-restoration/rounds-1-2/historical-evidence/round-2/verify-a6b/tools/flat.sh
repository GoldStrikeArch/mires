#!/usr/bin/env bash
# flat.sh: scripts/flat-result-test.sh (not a COMMON.md gate; it links one chunked program) with
# the cf80163 linker alone and with the linker under test alone, traced, to see whether its
# failure in this worktree depends on the linker. Prints each exit status and the last commands.
set -uo pipefail
source "$(dirname "$0")/env.sh"
cd "$W" || exit 2
JSC_LINK_BIN=$BASE_LINK SKIP_BUILD=1 bash -x scripts/flat-result-test.sh > "$V/logs/flat-result-base.log" 2>&1
echo "base exit $?"
SKIP_BUILD=1 bash -x scripts/flat-result-test.sh > "$V/logs/flat-result-fix.log" 2>&1
echo "fix exit $?"
for which in base fix; do
  echo "== $which, last traced commands"
  grep '^+' "$V/logs/flat-result-$which.log" | tail -3 | cut -c1-220
done
