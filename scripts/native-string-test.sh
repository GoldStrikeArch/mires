#!/usr/bin/env bash
# Checks that ordinary Rust String operations form one executable JS-native component.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/38_native_string.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

# The receipt runs on the jir lane (js-lane=jir) by default: W5h deleted the v1 text
# optimizer, so the jir representation planner is the only lowerer and the byte budgets
# below are the frozen v1-baseline receipts it must hold (shrink plus the absolute v1
# number +2%). NATIVE_LANE=baseline checks the baseline lane instead, where `js-opt=on`
# no longer rewrites anything: `on` must equal `off` byte for byte.
lane="${NATIVE_LANE:-jir}"

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off report on; do
	args="js-opt=$mode js-opt-passes=native-representations"
	if [ "$lane" = jir ]; then
		args="js-lane=jir $args"
	fi
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args js-opt-warnings=off js-opt-report=$work/$mode.json" \
		"$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
done
cmp "$work/off.js" "$work/report.js"

if [ "$lane" = baseline ]; then
	# The v1 text optimizer is deleted: `js-opt=on` on the baseline lane no longer rewrites
	# anything, so the receipt is byte-frozen -- `on` must equal `off` exactly.
	cmp "$work/off.js" "$work/on.js"
	echo "native String lowering (baseline lane): frozen, on == off ($(wc -c <"$work/off.js" | tr -d ' ') bytes)"
else
	off_bytes=$(wc -c <"$work/off.js" | tr -d ' ')
	on_bytes=$(wc -c <"$work/on.js" | tr -d ' ')
	if ((on_bytes * 4 >= off_bytes)); then
		echo "native String lowering (jir lane) did not remove at least 75%: $off_bytes -> $on_bytes" >&2
		exit 1
	fi
	# Phase 5 (W5a): the lane must reproduce the frozen v1-baseline receipt — the D11
	# deletion criterion. Landing under it is a pass and is reported as such.
	target=6508
	high=$((target * 102 / 100))
	if ((on_bytes > high)); then
		echo "native String lowering (jir lane) missed the v1 receipt: $on_bytes > $high bytes (v1 $target +2%)" >&2
		exit 1
	fi
	echo "native String lowering (jir lane): $off_bytes -> $on_bytes bytes (v1 receipt $target)"
fi
