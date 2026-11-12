#!/usr/bin/env bash
# mires.sh <src.rs> <mode> <outdir> [chunk:entry ...]
# Compiles a probe with the worktree's compiler in one mode and runs it.
# Modes: def (default profile), jiroff (js-lane=jir js-opt=off), rb (release-balanced),
#        rbz (rb + JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on), rbnc (rb + js-cleanup=off).
# With chunk:entry pairs: ES modules, one js-chunk per pair, run through scripts/run-chunks.sh
# calling each entry in the order given. Without: plain script, shim + program + entry.js.
# LINK=base uses the saved base jsc-link (same dylib and sysroot; the change is linker-only).
# Writes <outdir>/<name>.<mode>[.<tag>].{js,log,stdout,stderr,exit}; TAG env names the chunk run.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
W=${ROOT:-/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-aa8b8ec6c06479cb8}
src=$1; mode=$2; out=$3; shift 3
name=$(basename "$src" .rs)
mkdir -p "$out"
unset JSC_RUST_OPT_LEVEL JSC_RUST_MIR_INLINE
case "$mode" in
  def) args="" ;;
  jiroff) args="js-lane=jir js-opt=off" ;;
  rb) args="js-profile=release-balanced" ;;
  rbnc) args="js-profile=release-balanced js-cleanup=off" ;;
  rbz) args="js-profile=release-balanced"; export JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on ;;
  *) echo "unknown mode $mode"; exit 2 ;;
esac
# EXTRA: more backend options (link-only ones are safe); CALLS: the chunk:entry pairs to RUN, when
# the chunks were requested in another order.
if [ -n "${EXTRA:-}" ]; then args="$args $EXTRA"; fi
if [ "${LINK:-fix}" = base ]; then export JSC_LINK_BIN=$A/main-src/target/release/jsc-link; fi
tag=${TAG:-}
base=$out/$name.$mode${tag:+.$tag}
if [ "$#" -gt 0 ]; then
  chunkargs=""; for p in "$@"; do chunkargs="$chunkargs js-chunk=$p"; done
  args="$args js-modules=esm$chunkargs"
  # Sysroot keys on emit args only; js-modules and js-chunk are link options.
  export JSC_SYSROOT_ARGS="${args}"
fi
export JS_EXTRA_ARGS="$args"
rm -rf "$base.d"; mkdir -p "$base.d"
if ! (cd "$W" && SKIP_BUILD=1 bash scripts/compile_core.sh "$src" "$base.d/program.js") > "$base.log" 2>&1; then
  echo "COMPILE-FAIL" > "$base.stdout"; : > "$base.stderr"; echo compile > "$base.exit"
  exit 0
fi
if [ "$#" -gt 0 ]; then
  (cd "$W" && bash scripts/run-chunks.sh "$base.d/program.js" "va6-$name-$mode${tag:+-$tag}" ${CALLS:-"$@"}) > "$base.stdout" 2> "$base.stderr"
else
  cat "$W/runtime/shim.js" "$base.d/program.js" "$W/scripts/entry.js" | node - > "$base.stdout" 2> "$base.stderr"
fi
echo "$?" > "$base.exit"
