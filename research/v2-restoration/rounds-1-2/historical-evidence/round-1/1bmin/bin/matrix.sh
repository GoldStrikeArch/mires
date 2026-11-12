#!/usr/bin/env bash
# matrix.sh <tree> <arm> [modes...]: every fixture of every set in every mode, sequentially.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
tree=$1; arm=$2; shift 2
modes=${*:-rb rbopt0 rbco jir0 def}
start=$(date +%s)
for m in $modes; do
  for s in r1 r3new fuse; do
    bash $V/bin/mx.sh $tree $arm $m $s
  done
done
echo "MATRIX-DONE $arm $(( $(date +%s) - start ))s"
