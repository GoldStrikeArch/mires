#!/usr/bin/env bash
# mutants.sh <which>...: for each mutant (tools/mutate.py), builds jsc-link in the scratch copy
# mut-src (e7880e0's tree, the worktree's dylib copied in, its sysroots linked) and runs the
# COMMITTED tests against it:
#   unit      cargo test -p jsc-link --lib link::              (part of scripts/check.sh)
#   chunks    scripts/chunk-test.sh, js-lane=jir js-opt=on     (what scripts/check.sh runs)
#   fx-jir    scripts/test.sh 260, 261, 267, 268, js-lane=jir js-opt=on   (what CI runs)
# (The first three rows of mut/summary.txt, from an earlier run of this script, also ran the four
# fixtures under release-balanced + minify, the COMMON.md gate.)
# One row per mutant in mut/summary.txt; logs in mut/; each mutant's binary in mut/bin-<which>/.
set -uo pipefail
source "$(dirname "$0")/env.sh"
R=$V/mut-src
M=$V/mut
fixtures="260_static_cycles 261_static_cycle_cells 267_static_cycle_arrays 268_static_cycle_proto_key"
for which in "$@"; do
  python3 "$V/tools/mutate.py" "$which" "$M/pristine" "$R" > "$M/$which.mutate.log" 2>&1 || { echo "$which MUTATE-FAIL $(tail -1 "$M/$which.mutate.log")" | tee -a "$M/summary.txt"; continue; }
  (cd "$R" && cargo build --release -p jsc-link --locked) > "$M/$which.build.log" 2>&1 || { echo "$which BUILD-FAIL" | tee -a "$M/summary.txt"; continue; }
  mkdir -p "$M/bin-$which"; cp "$R/target/release/jsc-link" "$M/bin-$which/jsc-link"
  (cd "$R" && cargo test -p jsc-link --locked --lib link::) > "$M/$which.unit.log" 2>&1
  unit="$(grep -m1 -oE 'test result: [a-zA-Z]+\. [0-9]+ passed; [0-9]+ failed' "$M/$which.unit.log") [$(grep -E '^test .* FAILED$' "$M/$which.unit.log" | sed 's/^test //; s/ \.\.\. FAILED$//; s/.*:://' | tr '\n' ',')]"
  (cd "$R" && SKIP_BUILD=1 JS_EXTRA_ARGS='js-lane=jir js-opt=on' bash scripts/chunk-test.sh) > "$M/$which.chunks.log" 2>&1
  ch="$(grep -c '^PASS' "$M/$which.chunks.log") pass, $(grep -c '^FAIL' "$M/$which.chunks.log") fail: $(grep '^FAIL' "$M/$which.chunks.log" | sed 's/^FAIL  //; s/\].*/]/' | tr '\n' ';')"
  row="$which | unit: $unit | chunks: $ch"
  : > "$M/$which.fx-jir.log"
  for f in $fixtures; do
    (cd "$R" && SKIP_BUILD=1 JS_EXTRA_ARGS='js-lane=jir js-opt=on' bash scripts/test.sh "$f") >> "$M/$which.fx-jir.log" 2>&1
  done
  fx=$(grep -E '^core/26[0178]_[a-z_]+ +(PASS|FAIL)' "$M/$which.fx-jir.log" | awk '{printf "%s=%s(%s) ", substr($1,6,3), $2, $3}')
  echo "$row | fx-jir: $fx" | tee -a "$M/summary.txt"
done
python3 "$V/tools/mutate.py" none "$M/pristine" "$R" > /dev/null
