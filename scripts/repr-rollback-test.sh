#!/usr/bin/env bash
#
# The representation commit's rollback path (Phase 4, D4): `RCGJS_REPR_FAULT=verify` forces every
# region's commit-time verification to fail, so the regions decline atomically and the originals
# are kept — the `abi-retry-test.sh` pattern for phase B1.
#
# What is asserted is the rollback being *complete*:
#   * without the fault, fixture 74 emits a validated representation region;
#   * with the fault, stdout is still the fixture's own `.expected`, the sidecar records the
#     `version.carrier-commit-rollback` decline and commits nothing;
#   * the faulted program is byte for byte the one a build without the `native-representations`
#     pass prints — the plan left no trace behind.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture=74_repr_rollback
out="$root/build/core/$fixture.js"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi
JS_EXTRA_ARGS='js-opt=on' SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null

JS_EXTRA_ARGS='js-lane=jir js-opt=on' SKIP_BUILD=1 "$script_dir/test.sh" "$fixture" >"$work/commit.log" 2>&1 ||
	{ cat "$work/commit.log" >&2 && exit 1; }
committed=$(node -e "console.log(JSON.parse(require('fs').readFileSync('$out.lane.json','utf8')).repr.final_emitted.applied)")
if [ "$committed" -lt 1 ]; then
	echo "repr rollback: the fixture commits nothing without the fault (committed=$committed)" >&2
	exit 1
fi

RCGJS_REPR_FAULT=verify JS_EXTRA_ARGS='js-lane=jir js-opt=on' SKIP_BUILD=1 \
	"$script_dir/test.sh" "$fixture" >"$work/fault.log" 2>&1 ||
	{ cat "$work/fault.log" >&2 && exit 1; }
cp "$out" "$work/fault.js"
node - "$out.lane.json" <<'NODE'
const repr = JSON.parse(require("node:fs").readFileSync(process.argv[2], "utf8")).repr;
if (repr.final_emitted.applied !== 0 || repr.validated.would_commit !== 0 ||
    repr.validated.adapters !== 0 || repr.validated.versions !== 0) {
  throw new Error(`the fault left a commit behind: ${JSON.stringify(repr)}`);
}
const rolled = repr.decisions.filter(
  (d) => (d.decline ?? "").startsWith("version.carrier-commit-rollback"),
);
if (rolled.length === 0) {
  throw new Error(`no decision records the rollback: ${JSON.stringify(repr.decisions)}`);
}
if (repr.validated.cost_positive < rolled.length) {
  throw new Error("a rolled-back region is not counted cost-positive");
}
console.log(`  rollback recorded on ${rolled.length} region(s), committed 0`);
NODE

# The plan left no trace: the faulted program is the one a build without the pass prints.
JS_EXTRA_ARGS='js-lane=jir js-opt=on js-opt-passes=sroa,ast-dce,future-poller,link-dedup' \
	SKIP_BUILD=1 "$script_dir/test.sh" "$fixture" >"$work/no-pass.log" 2>&1 ||
	{ cat "$work/no-pass.log" >&2 && exit 1; }
cmp "$work/fault.js" "$out"

echo "repr rollback: PASS ($fixture declined atomically and printed the pass-less program byte for byte)"
