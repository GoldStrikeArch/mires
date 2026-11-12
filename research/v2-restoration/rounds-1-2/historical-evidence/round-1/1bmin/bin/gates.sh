#!/usr/bin/env bash
# gates.sh <tree> <arm>: the script gates, sequentially, SKIP_BUILD=1.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
tree=$1; arm=$2
cd $tree
for g in cleanup-smoke.sh cleanup-admission-test.sh noop-landing-pads-test.sh active-poll-cleanup-test.sh async-test.sh extern-test.sh; do
  start=$(date +%s)
  SKIP_BUILD=1 bash scripts/$g > $V/logs/gate-$arm-$g.log 2>&1
  echo "gate $arm $g exit=$? $(( $(date +%s) - start ))s | $(tail -1 $V/logs/gate-$arm-$g.log | cut -c1-150)"
done
echo GATES-DONE $arm
