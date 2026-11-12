#!/usr/bin/env bash
# lean-ab.sh <tag> <workers> <JS_EXTRA_ARGS> [env assignments...]: compiles every program in
# examples/core-tests (prelude.rs aside) once, in parallel, with every link done by both linkers
# (tools/ab-link.sh), without running anything. For byte identity only: ab2/<tag>/ab.log gets one
# line per link. A fixture that needs `--extern` or must fail to compile fails with both linkers
# or before the link; those are listed in ab2/<tag>/nolink.txt.
set -uo pipefail
source "$(dirname "$0")/env.sh"
tag=$1; workers=$2; args=$3; shift 3
for kv in "$@"; do export "${kv?}"; done
export AB_DIR=$V/ab2/$tag
rm -rf "$AB_DIR"; mkdir -p "$AB_DIR/out"
export JSC_LINK_BIN=$V/tools/ab-link.sh JS_EXTRA_ARGS="$args"
# Compiles run in parallel; nothing builds a sysroot in this worktree while they do.
export RCGJS_INTEGRATION_LOCK_HELD=$W/build/.integration-lock
cd "$W" || exit 2
ls examples/core-tests/*.rs | grep -v '/prelude.rs$' | xargs -P "$workers" -n 1 bash "$V/tools/lean-one.sh"
sort -o "$AB_DIR/failed.txt" "$AB_DIR/failed.txt" 2>/dev/null
echo "$tag: programs $(ls examples/core-tests/*.rs | grep -vc '/prelude.rs$'); links: $(awk '{print $1}' "$AB_DIR/ab.log" | sort | uniq -c | tr '\n' ' '); compile failures: $(grep -c . "$AB_DIR/failed.txt" 2>/dev/null)"
