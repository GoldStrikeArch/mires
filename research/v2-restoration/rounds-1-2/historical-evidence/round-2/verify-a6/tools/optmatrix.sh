#!/usr/bin/env bash
# optmatrix.sh <tag> <probe>... : each probe under a list of backend option sets (one per line of
# $A/tools/optsets.txt: `<label>|<JS_EXTRA_ARGS>`), in $ROOT. The matching sysroot is built first.
# An option set containing js-modules=esm is run through scripts/run-esm.sh. With js-source-map=on
# the map is checked with scripts/check-sourcemap.js as well.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
W=${ROOT:?ROOT}
tag=$1; shift
out=$A/out/$tag
mkdir -p "$out"
while IFS='|' read -r label args; do
  [ -z "$label" ] && continue
  case "$label" in \#*) continue;; esac
  (cd "$W" && SKIP_BUILD=1 JS_EXTRA_ARGS="$args" bash scripts/build_sysroot.sh > "$out/sysroot.$label.log" 2>&1) || { echo "$label SYSROOT-FAIL" | tee -a "$out/summary.txt"; continue; }
  for name in "$@"; do
    src=$A/src/$name.rs
    nat=$(bash "$A/tools/native.sh" "$src") || { echo "$name NATIVE-FAIL" | tee -a "$out/summary.txt"; continue; }
    base=$out/$name.$label
    rm -rf "$base.d"; mkdir -p "$base.d"
    if ! (cd "$W" && SKIP_BUILD=1 JS_EXTRA_ARGS="$args" bash scripts/compile_core.sh "$src" "$base.d/program.js") > "$base.log" 2>&1; then
      echo "$name $label COMPILE-FAIL $(grep -m1 -E '^error' "$base.log" | cut -c1-300)" | tee -a "$out/summary.txt"; continue
    fi
    case "$args" in
      *js-modules=esm*) (cd "$W" && bash scripts/run-esm.sh "$base.d/program.js" "va6-$name-$label") > "$base.stdout" 2> "$base.stderr" ;;
      *) cat "$W/runtime/shim.js" "$base.d/program.js" "$W/scripts/entry.js" | node - > "$base.stdout" 2> "$base.stderr" ;;
    esac
    code=$?
    extra=""
    case "$args" in *js-source-map=on*) node "$W/scripts/check-sourcemap.js" "$base.d/program.js" > "$base.map.log" 2>&1; extra=" map-check=$?" ;; esac
    cyc=$(grep -c '^Object.assign(' "$base.d/program.js")
    if [ "$code" = 0 ] && cmp -s "$nat" "$base.stdout"; then
      echo "$name $label PASS fills=$cyc$extra" | tee -a "$out/summary.txt"
    else
      echo "$name $label FAIL exit=$code fills=$cyc$extra $(grep -m1 -E 'Error' "$base.stderr" | cut -c1-200)" | tee -a "$out/summary.txt"
    fi
  done
done < "$A/tools/optsets.txt"
