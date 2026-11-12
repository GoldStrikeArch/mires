#!/usr/bin/env bash
# whole.sh <probe name> <mode>...: one probe (src/<name>.rs) UNSPLIT, linked by the cf80163 linker
# and by the linker under test, run as a plain script (shim + program + scripts/entry.js) and
# compared with the native run. One line per mode and linker:
#   <name> <mode> <base|fix>: REFUSED <first error> | OK | WRONG (n lines differ) | THROWS <error>
# and whether the two programs are byte-identical.
set -uo pipefail
source "$(dirname "$0")/env.sh"
name=$1; shift
src=$V/src/$name.rs
nat=$(bash "$A6/tools/native.sh" "$src" "$V/native/$name") || { echo "$name NATIVE-FAIL: $nat"; exit 1; }
for mode in "$@"; do
  keep=$V/keep/$name.$mode.whole
  bash "$V/tools/cc.sh" "$src" "$mode" "$keep" || { echo "$name $mode: nothing captured"; continue; }
  for which in base fix; do
    out=$V/out/whole/$name.$mode/$which
    bash "$V/tools/ln.sh" "$keep" "$which" "$out"
    if [ "$(cat "$out/status")" != 0 ]; then
      echo "$name $mode $which: REFUSED $(grep -m1 '^error' "$out/link.log" | cut -c1-260)"
      continue
    fi
    cat "$W/runtime/shim.js" "$out/program.js" "$W/scripts/entry.js" | node - > "$out/stdout" 2> "$out/stderr"
    status=$?
    if [ "$status" -ne 0 ]; then
      echo "$name $mode $which: THROWS (exit $status) $(grep -m1 Error "$out/stderr" | cut -c1-160)"
    elif cmp -s "$nat" "$out/stdout"; then
      echo "$name $mode $which: OK"
    else
      echo "$name $mode $which: WRONG ($(diff "$nat" "$out/stdout" | grep -c '^[<>]') diff lines; native: $(tr '\n' ' ' < "$nat" | cut -c1-80) | mires: $(tr '\n' ' ' < "$out/stdout" | cut -c1-80))"
    fi
  done
  b=$V/out/whole/$name.$mode/base/program.js; f=$V/out/whole/$name.$mode/fix/program.js
  if [ -f "$b" ] && [ -f "$f" ]; then cmp -s "$b" "$f" && echo "$name $mode: byte-identical" || echo "$name $mode: PROGRAMS DIFFER"; fi
done
