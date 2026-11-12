#!/usr/bin/env bash
# abrun.sh <tag> <command...>: runs a suite command from the worktree root with every link done by
# both linkers (tools/ab-link.sh). The suite's own output goes to ab2/<tag>/suite.log (with its
# exit status on the last line); the per-link comparison to ab2/<tag>/ab.log; a count is printed.
set -uo pipefail
source "$(dirname "$0")/env.sh"
tag=$1; shift
export AB_DIR=$V/ab2/$tag
rm -rf "$AB_DIR"; mkdir -p "$AB_DIR"
export JSC_LINK_BIN=$V/tools/ab-link.sh
cd "$W" || exit 2
"$@" > "$AB_DIR/suite.log" 2>&1
echo "exit $?" >> "$AB_DIR/suite.log"
echo "$tag: $(tail -1 "$AB_DIR/suite.log"); links: $(awk '{print $1}' "$AB_DIR/ab.log" 2>/dev/null | sort | uniq -c | tr '\n' ' ')"
