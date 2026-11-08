#!/usr/bin/env bash
#
# The per-row closure semantics under every JIR optimizer mode. The ordinary DOM trace proves
# where handlers are installed; for-handler-check.mjs invokes them and proves which iteration each
# one captured.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
log_dir="$root/build/logs"
mkdir -p "$log_dir"

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null || exit 1
fi

passed=0
status=0
base_args="js-names=readable js-comments=on js-modules=esm js-dom-module=./topcoat-dom.js js-lane=jir"
for mode in off report on; do
	args=$base_args
	[ "$mode" = off ] || args="$args js-opt=$mode"
	printf '\n==> for-row handlers (js-lane=jir, js-opt=%s)\n' "$mode"
	if ! JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" \
		>"$log_dir/for-handler-$mode-sysroot.log" 2>&1; then
		cat "$log_dir/for-handler-$mode-sysroot.log"
		status=1
		continue
	fi
	if DOM_OPT_MODE="$mode" JS_EXTRA_ARGS='js-lane=jir' SKIP_BUILD=1 \
		"$script_dir/dom-test.sh" 23_for_row_handlers; then
		passed=$((passed + 1))
	else
		status=1
	fi
done

printf '\n%d/3 passed\n' "$passed"
exit "$status"
