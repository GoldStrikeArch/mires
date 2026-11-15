#!/usr/bin/env bash
#
# The flat direct-call ABI's retry path: one rewritten item declines, so the whole lane run
# restarts with an empty plan and the program comes out in the baseline ABI.
#
# `$JSC_JIR_ABI_FAULT` (see `jsc-link/src/link/lane.rs::abi_fault`) is the only way to reach that
# path on purpose -- it makes the item it names decline in phase C, after the plan rewrote it.
# What is asserted is the fallback being *complete*: the fixture still prints its own `.expected`,
# the sidecar says the run retried once and kept no register, nothing fell back to v1 text, and
# the program is byte for byte the one `js-jir-flat-abi=off` prints.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture=91_flat_result
out="$root/build/core/$fixture.js"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

JSC_JIR_ABI_FAULT=make_pair JS_EXTRA_ARGS='js-lane=jir js-opt=on' \
	"$script_dir/test.sh" "$fixture" >"$work/fault.log" 2>&1 ||
	{ cat "$work/fault.log" >&2 && exit 1; }
cp "$out" "$work/fault.js"
sidecar=$(tr -d ' \n' <"$out.lane.json")
for want in '"plan_retries":1' '"registers":0' '"fallback_count":0'; do
	case $sidecar in
	*"$want"*) ;;
	*)
		echo "abi retry: sidecar does not say $want" >&2
		exit 1
		;;
	esac
done

JS_EXTRA_ARGS='js-lane=jir js-opt=on js-jir-flat-abi=off' "$script_dir/test.sh" "$fixture" >"$work/flat-off.log" 2>&1 ||
	{ cat "$work/flat-off.log" >&2 && exit 1; }
cmp "$work/fault.js" "$out"

echo "flat ABI retry: PASS ($fixture retried once and printed the baseline ABI byte for byte)"
