#!/usr/bin/env bash
# chain.sh: everything after the final test.sh, serially: shim mutants, the remaining repository
# gates, both packages, the kernel check, the probes, the paired timing.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/verify-granule-min/probe/src
step() { echo "== $1 $(date +%T)"; }
step "shim mutants"
mkdir -p "$G/r2cfix" && cp "$G"/r2c-core-out/12[234]_*.js "$G/r2cfix/"
bash "$G/run-mutants2.sh" "$G/r2cfix" >"$G/mutants2c.log" 2>&1
grep -E "^== |^✖|fixture|ℹ fail" "$G/mutants2c.log" | grep -v "failing tests"
step "gates"
bash "$G/gates2.sh" >"$G/gates2c.log" 2>&1
cat "$G/gates2c.log"
step "packages"
bash "$G/packages.sh" r2 >"$G/packages-r2.log" 2>&1
cat "$G/packages-r2.log"
cd "$W" || exit 1
node experiments/fingerprintjs-rust-compact/tools/build.mjs --profile release-balanced --optimizer on --lane jir \
	--cleanup on --scheduler boundary --lane-report --output experiments/fingerprintjs-rust-compact/dist/granule-min \
	>"$G/fp-gate-build-r2.log" 2>&1
echo "fingerprint gate build exit $?"
node experiments/tools/check-oss-kernels.mjs experiments/fingerprintjs-rust-compact/dist/granule-min/fp.esm.min.js \
	"$G/kernels-r2.json" >"$G/kernels-r2.log" 2>&1
echo "kernels exit $?: $(tail -1 "$G/kernels-r2.log")"
step "probes"
bash "$G/compile2.sh" r2 "$V" "$G/probe/r2c" repro var_chain var_ret var_ptr str3 arg3 acc_vec acc_str leak_arg leak_sarg
R2_DIR=$G/probe/r2c node "$G/fourway.mjs" repro var_chain var_ret var_ptr str3 arg3
for n in acc_vec acc_str leak_arg leak_sarg; do
	echo "$n: $(node "$G/count.mjs" "$W/runtime/shim.js" "$G/probe/r2c/$n.js")"
done
step "timing"
(cd "$G" && NEW_LABEL=r2 node ab.mjs 12 2>"$G/ab-r2-progress.log")
step "done"
