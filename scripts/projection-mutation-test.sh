#!/usr/bin/env bash
# Checks that return-field forwarding does not cross a mutation of the returned aggregate.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/92_projection_mutation.rs"
expected="$root/examples/core-tests/92_projection_mutation.expected"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

for mode in off on; do
	args="js-lane=jir js-opt=$mode"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JSC_RUST_MIR_INLINE=off JS_EXTRA_ARGS="$args" \
		"$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
	cat "$root/runtime/shim.js" "$work/$mode.js" "$script_dir/entry.js" | node - |
		diff -u "$expected" -
done

echo "return projection mutation boundary (jir lane): PASS"
