#!/usr/bin/env bash
# mires.sh <root> <src.rs> <mode> <outdir>
set -uo pipefail
W=$1; src=$2; mode=$3; out=$4
name=$(basename "$src" .rs)
mkdir -p "$out"
unset JSC_RUST_OPT_LEVEL JSC_RUST_MIR_INLINE
case "$mode" in
  def) export JS_EXTRA_ARGS="" ;;
  jiroff) export JS_EXTRA_ARGS="js-lane=jir js-opt=off" ;;
  rb) export JS_EXTRA_ARGS="js-profile=release-balanced" ;;
  rbz) export JS_EXTRA_ARGS="js-profile=release-balanced"; export JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on ;;
  *) echo "unknown mode $mode"; exit 2 ;;
esac
base=$out/$name.$mode
if ! (cd "$W" && SKIP_BUILD=1 bash scripts/compile_core.sh "$src" "$base.js") > "$base.log" 2>&1; then
  echo "COMPILE-FAIL" > "$base.stdout"; : > "$base.stderr"; echo compile > "$base.exit"
  exit 0
fi
cat "$W/runtime/shim.js" "$base.js" "$W/scripts/entry.js" | node - > "$base.stdout" 2> "$base.stderr"
echo "$?" > "$base.exit"
