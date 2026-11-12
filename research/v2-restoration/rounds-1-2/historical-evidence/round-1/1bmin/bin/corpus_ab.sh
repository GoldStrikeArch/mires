#!/usr/bin/env bash
# corpus_ab.sh <runs-per-arm> <samples> <warmups> <tag>: alternates the saved base/mine corpus
# builds into the worktree's experiments/browser-workloads/dist and runs the pure Node harness.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
runs=${1:-4}; samples=${2:-9}; warmups=${3:-2}; tag=${4:-pure}
R=$V/corpus-runs/$tag; mkdir -p $R
D=$W/experiments/browser-workloads
cd $W
for r in $(seq 1 $runs); do
  if [ $((r % 2)) = 1 ]; then order="base final3"; else order="final3 base"; fi
  for arm in $order; do
    rm -rf $D/dist; cp -R $V/corpus-dist/$arm/dist $D/dist
    mkdir -p $D/results; cp $V/corpus-dist/$arm/results/build.json $D/results/build.json
    uptime > $R/$arm-r$r.uptime
    NODE_OPTIONS=--preserve-symlinks node $D/tools/run.mjs --pure --samples $samples --warmups $warmups --output $R/$arm-r$r.json > $R/$arm-r$r.log 2>&1
    echo "$tag run $r $arm exit=$?"
  done
done
echo CORPUS-AB-DONE $tag
