#!/usr/bin/env bash
# Checks native Vec swapping through ordinary safe Rust.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/core-tests/43_native_vec_swap.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

# The suite runs on the jir lane (js-lane=jir) by default: W5h deleted the v1 text
# optimizer, so the jir lane is the only lowerer. NATIVE_LANE=baseline checks the
# baseline lane instead, where `js-opt=on` no longer rewrites anything: `on` must
# equal `off` byte for byte.
lane="${NATIVE_LANE:-jir}"

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

# The behaviour half needs `on` to expose the accessors over a committed representation,
# which only the jir lane still does.
if [ "$lane" = jir ]; then
node - "$work" "$root" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const work = process.argv[2];
const root = process.argv[3];
const context = vm.createContext({ TextEncoder });
vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
vm.runInContext(fs.readFileSync(path.join(work, "on.js"), "utf8"), context);

let values = context.native_vec_swap_make();
const read = () => [0, 1, 2].map((index) => context.native_vec_swap_get(values, index));
const shape = () => [
  context.native_vec_swap_len(values),
  context.native_vec_swap_capacity(values),
];

context.native_vec_swap(values, 0, 2);
if (read().join() !== "8,5,3" || shape().join() !== "3,5") {
  throw new Error("native Vec swap moved the wrong values or changed its shape");
}
context.native_vec_swap(values, 1, 1);
if (read().join() !== "8,5,3") {
  throw new Error("native Vec same-index swap changed its value");
}

for (const swap of [
  () => context.native_vec_swap(values, 3, 0),
  () => context.native_vec_swap(values, 0, 3),
]) {
  let boundsError;
  try {
    swap();
  } catch (error) {
    boundsError = error;
  }
  // v1's text kernel threw its own RangeError here; the jir lane keeps Rust's panic path.
  // The contract is: an out-of-bounds swap throws and leaves the vector untouched.
  if (boundsError === undefined) {
    throw new Error("out-of-bounds swap did not throw");
  }
  if (read().join() !== "8,5,3" || shape().join() !== "3,5") {
    throw new Error("failed native Vec swap changed the vector");
  }
}

const original = values;
values = context.native_vec_swap_round_trip(values);
if (values !== original || read().join() !== "8,5,3") {
  throw new Error("native Vec swap did not survive an opaque same-artifact round trip");
}

NODE
fi

if [ "$lane" = baseline ]; then
	# The v1 text optimizer is deleted: `js-opt=on` on the baseline lane no longer rewrites
	# anything, so the receipt is byte-frozen -- `on` must equal `off` exactly.
	cmp "$work/off.js" "$work/on.js"
	echo "native Vec swap (baseline lane): frozen, on == off ($(wc -c <"$work/off.js" | tr -d ' ') bytes)"
else
	off_bytes=$(wc -c <"$work/off.js" | tr -d ' ')
	on_bytes=$(wc -c <"$work/on.js" | tr -d ' ')
	if ((on_bytes >= off_bytes)); then
		echo "native Vec swap (jir lane) did not shrink: $off_bytes -> $on_bytes" >&2
		exit 1
	fi
	echo "native Vec swap (jir lane): PASS ($off_bytes -> $on_bytes bytes)"
fi
