#!/usr/bin/env bash
# mutants.sh <which>... : for each mutant, build jsc-link in the scratch copy mut-src and run the
# COMMITTED tests against it: the jsc-link unit tests of link:: (cargo test), fixtures 260/261
# through scripts/test.sh and scripts/chunk-test.sh, in the default profile and release-balanced.
# One summary row per mutant in mut/summary.txt; logs in mut/.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
R=$A/mut-src
M=$A/mut
mkdir -p "$M/pristine"
if [ ! -f "$M/pristine/statics.rs" ]; then
  cp "$A/fix-src/jsc-link/src/link/statics.rs" "$M/pristine/statics.rs"
  cp "$A/fix-src/jsc-link/src/link.rs" "$M/pristine/link.rs"
fi
for which in "$@"; do
  python3 -I "$A/tools/mutate.py" "$which" "$M/pristine" "$R" > "$M/$which.mutate.log" 2>&1 || { echo "$which MUTATE-FAIL $(tail -1 "$M/$which.mutate.log")" | tee -a "$M/summary.txt"; continue; }
  (cd "$R" && cargo build --release -p jsc-link --locked) > "$M/$which.build.log" 2>&1 || { echo "$which BUILD-FAIL" | tee -a "$M/summary.txt"; continue; }
  (cd "$R" && cargo test --release -p jsc-link --locked link::) > "$M/$which.unit.log" 2>&1
  unit="unit: $(grep -E '^test result' "$M/$which.unit.log" | head -1 | sed 's/test result: //; s/; [0-9]* ignored.*//') [$(grep -E '^test .* FAILED' "$M/$which.unit.log" | sed 's/^test //; s/ \.\.\. FAILED//; s/link::statics::tests:://; s/link::tests:://' | tr '\n' ',')]"
  row="$which | $unit"
  for mode in def rb; do
    case $mode in rb) args='js-profile=release-balanced' ;; def) args='' ;; esac
    (cd "$R" && SKIP_BUILD=1 JS_EXTRA_ARGS="$args" bash scripts/test.sh 260_static_cycles 261_static_cycle_cells) > "$M/$which.$mode.fixtures.log" 2>&1
    fx=$(grep -E '^core/26[01]_\S+\s+(PASS|FAIL)' "$M/$which.$mode.fixtures.log" | awk '{printf "%s=%s ", substr($1,6,3), $2}')
    (cd "$R" && SKIP_BUILD=1 JS_EXTRA_ARGS="$args" bash scripts/chunk-test.sh) > "$M/$which.$mode.chunks.log" 2>&1
    ch=$(grep -E '^(PASS|FAIL)' "$M/$which.$mode.chunks.log" | awk '{print $1}' | sort | uniq -c | awk '{printf "%s%s ", $1, $2}')
    row="$row | $mode: $fx chunks: $ch"
  done
  echo "$row" | tee -a "$M/summary.txt"
done
python3 -I "$A/tools/mutate.py" none "$M/pristine" "$R" > /dev/null
