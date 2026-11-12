#!/usr/bin/env bash
# corpus_main.sh: build the ten-workload corpus with the main (ea5604f) compiler in the plain source
# tree tree-main2 (a revision stub stands in for the repository query the receipt makes), then
# compare each workload's minified and raw program bytes with the fix build saved in corpus-out/fix.
set -uo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
chmod +x "$V"/fakegit/*
cd "$V/tree-main2" || exit 1
PATH="$V/fakegit:$PATH" node experiments/browser-workloads/tools/build.mjs > "$V/logs/corpus-build-main.log" 2>&1
echo "build exit $?"
mkdir -p "$V/corpus-out/main"
for w in minisearch marked fflate idb idb_lines ky uplot explorer workbox workbox_direct; do
  cp "experiments/browser-workloads/dist/$w/index.min.js" "$V/corpus-out/main/$w.index.min.js"
  cp "experiments/browser-workloads/dist/$w/raw/program.js" "$V/corpus-out/main/$w.program.js"
  a=$(wc -c < "$V/corpus-out/main/$w.index.min.js"); b=$(wc -c < "$V/corpus-out/fix/$w.index.min.js")
  c=$(wc -c < "$V/corpus-out/main/$w.program.js"); d=$(wc -c < "$V/corpus-out/fix/$w.program.js")
  cmp -s "$V/corpus-out/main/$w.index.min.js" "$V/corpus-out/fix/$w.index.min.js" && m=SAME || m=DIFF
  cmp -s "$V/corpus-out/main/$w.program.js" "$V/corpus-out/fix/$w.program.js" && r=SAME || r=DIFF
  echo "$w min $m $a -> $b | raw $r $c -> $d"
done
