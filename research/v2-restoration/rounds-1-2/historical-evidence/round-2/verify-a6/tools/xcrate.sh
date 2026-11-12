#!/usr/bin/env bash
# xcrate.sh <tag>: a static cycle in a dependency rlib used by a client crate with its own cycle,
# in $ROOT, against the native run of the combined single-file version. Modes: def rb rbz.
# Also splits the cross-crate program into two chunks is not possible (one entry); single file only.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
W=${ROOT:?ROOT}
tag=$1
out=$A/out/$tag
mkdir -p "$out"
nat=$(bash "$A/tools/native.sh" "$A/src/x_combined.rs") || { echo "NATIVE-FAIL $nat"; exit 1; }
for mode in def rb rbz; do
  unset JSC_RUST_OPT_LEVEL JSC_RUST_MIR_INLINE
  case "$mode" in
    def) args="" ;;
    rb) args="js-profile=release-balanced" ;;
    rbz) args="js-profile=release-balanced"; export JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on ;;
  esac
  d=$out/$mode; rm -rf "$d"; mkdir -p "$d"
  if ! (cd "$W" && SKIP_BUILD=1 JS_EXTRA_ARGS="$args" bash scripts/compile_core.sh "$A/src/x_cyclib.rs" "$d/libcyclib.rlib" --crate-type rlib --crate-name cyclib) > "$d/lib.log" 2>&1; then
    echo "xcrate $mode LIB-COMPILE-FAIL $(grep -m1 '^error' "$d/lib.log" | cut -c1-200)" | tee -a "$out/summary.txt"; continue
  fi
  if ! (cd "$W" && SKIP_BUILD=1 JS_EXTRA_ARGS="$args" bash scripts/compile_core.sh "$A/src/x_client.rs" "$d/program.js" --extern "cyclib=$d/libcyclib.rlib" --search "$d") > "$d/client.log" 2>&1; then
    echo "xcrate $mode CLIENT-COMPILE-FAIL $(grep -m1 '^error' "$d/client.log" | cut -c1-300)" | tee -a "$out/summary.txt"; continue
  fi
  cat "$W/runtime/shim.js" "$d/program.js" "$W/scripts/entry.js" | node - > "$d/stdout" 2> "$d/stderr"
  code=$?
  if [ "$code" = 0 ] && cmp -s "$nat" "$d/stdout"; then
    echo "xcrate $mode PASS fills=$(grep -c '^Object.assign(' "$d/program.js")" | tee -a "$out/summary.txt"
  else
    echo "xcrate $mode FAIL exit=$code $(grep -m1 Error "$d/stderr" | cut -c1-200)" | tee -a "$out/summary.txt"
  fi
done
