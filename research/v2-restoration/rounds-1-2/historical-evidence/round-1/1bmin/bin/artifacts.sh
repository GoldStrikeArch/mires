#!/usr/bin/env bash
# artifacts.sh <arm>: with the worktree in the state of <arm> (base or mine), builds the compiler,
# compiles the benchmarks (rb, def), the compact package and the browser-workload corpus, and saves
# each artifact under 1bmin/ so later timing runs can interleave the two arms.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
arm=$1
cd $W
bash scripts/build.sh --locked > $V/logs/art-$arm-build.log 2>&1; echo "$arm build exit=$?"
shasum -a 256 target/release/librustc_codegen_mires.dylib
B=$V/bench; mkdir -p $B/out
for mode in rb def; do
  args=""; [ $mode = rb ] && args="js-profile=release-balanced"
  for b in bench_put bench2 bench3; do
    JS_EXTRA_ARGS="$args" SKIP_BUILD=1 bash scripts/compile_core.sh $B/$b.rs $B/out/$arm-$b-$mode.js > $B/out/$arm-$b-$mode.log 2>&1
    echo "compile $arm $b $mode exit=$? $(wc -c < $B/out/$arm-$b-$mode.js) bytes"
  done
done
cp runtime/shim.js $B/out/$arm-shim.js
# compact package
start=$(date +%s)
node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir --cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust-compact/dist/1bmin-$arm > $V/logs/art-$arm-compact.log 2>&1
echo "$arm compact exit=$? $(( $(date +%s) - start ))s"
rm -rf $V/compact/$arm; mkdir -p $V/compact
cp -R experiments/fingerprintjs-rust-compact/dist/1bmin-$arm $V/compact/$arm
# browser-workload corpus
start=$(date +%s)
node experiments/browser-workloads/tools/build.mjs > $V/logs/art-$arm-corpus.log 2>&1
echo "$arm corpus exit=$? $(( $(date +%s) - start ))s"
rm -rf $V/corpus-dist/$arm; mkdir -p $V/corpus-dist/$arm
cp -R experiments/browser-workloads/dist $V/corpus-dist/$arm/dist
mkdir -p $V/corpus-dist/$arm/results; cp experiments/browser-workloads/results/build.json $V/corpus-dist/$arm/results/ 2>/dev/null
echo ARTIFACTS-DONE $arm
