#!/usr/bin/env bash
# Checks idiomatic integer, bool, and char to_string lowering into native JS strings.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
immutable="$root/examples/core-tests/49_native_int_string.rs"
mutable="$root/examples/core-tests/50_native_int_string_mut.rs"
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
		"$script_dir/compile_core.sh" "$immutable" "$work/$mode.js" >/dev/null
	JS_EXTRA_ARGS="$args js-opt-warnings=off js-opt-report=$work/mut-$mode.json" \
		"$script_dir/compile_core.sh" "$mutable" "$work/mut-$mode.js" >/dev/null
done
cmp "$work/off.js" "$work/report.js"
cmp "$work/mut-off.js" "$work/mut-report.js"

node - "$work" "$root" "$lane" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const work = process.argv[2];
const root = process.argv[3];
const lane = process.argv[4];

// Every expectation is the exact text Rust's `Display` writes.
const integers = [
  ["native_i32_string", 0, "0"],
  ["native_i32_string", -1, "-1"],
  ["native_i32_string", 2147483647, "2147483647"],
  ["native_i32_string", -2147483648, "-2147483648"],
  ["native_u8_string", 0, "0"],
  ["native_u8_string", 255, "255"],
  ["native_usize_string", 0, "0"],
  ["native_usize_string", 4294967295, "4294967295"],
  ["native_i64_string", 0n, "0"],
  ["native_i64_string", -1n, "-1"],
  ["native_i64_string", 9223372036854775807n, "9223372036854775807"],
  ["native_i64_string", -9223372036854775808n, "-9223372036854775808"],
  ["native_u64_string", 0n, "0"],
  ["native_u64_string", 18446744073709551615n, "18446744073709551615"],
  ["native_bool_string", true, "true"],
  ["native_bool_string", false, "false"],
  ["native_char_string", 0x41, "A"],
  ["native_char_string", 0, "\0"],
  ["native_char_string", 0xe9, "\u00e9"],
  ["native_char_string", 0x4e2d, "\u4e2d"],
  ["native_char_string", 0x1f600, "\u{1f600}"],
  ["native_char_string", 0x10ffff, "\u{10ffff}"],
];

function utf8Length(text) {
  return Buffer.byteLength(text, "utf8");
}

function load(file) {
  const context = vm.createContext({ TextDecoder, TextEncoder });
  vm.runInContext(fs.readFileSync(path.join(root, "runtime/shim.js"), "utf8"), context);
  vm.runInContext(fs.readFileSync(file, "utf8"), context);
  return context;
}

for (const mode of ["off", "report", "on"]) {
  const context = load(path.join(work, `${mode}.js`));
  for (const [symbol, input, expected] of integers) {
    const value = context[symbol](input);
    const actual = context.native_int_string_as_str(value);
    if (actual !== expected) {
      throw new Error(
        `${mode}: ${symbol}(${input}) formatted as ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`,
      );
    }
    if (context.native_int_string_len(value) !== utf8Length(expected)) {
      throw new Error(`${mode}: ${symbol}(${input}) has the wrong UTF-8 byte length`);
    }
    // A committed immutable String is a JS string; only the jir lane still commits.
    if (lane === "jir" && mode === "on" && typeof value !== "string") {
      throw new Error(`${mode}: ${symbol}(${input}) did not use an immutable native String`);
    }
  }

  const mutated = load(path.join(work, `mut-${mode}.js`));
  const pushed = [
    ["native_int_string_pushed", -12, 0x21, "-12!"],
    ["native_int_string_pushed", 2147483647, 0x1f600, "2147483647\u{1f600}"],
    ["native_char_string_pushed", 0x4e2d, 0xe9, "\u4e2d\u00e9"],
  ];
  for (const [symbol, value, suffix, expected] of pushed) {
    const built = mutated[symbol](value, suffix);
    const actual = mutated.native_mut_string_as_str(built);
    if (actual !== expected) {
      throw new Error(
        `${mode}: ${symbol}(${value}, ${suffix}) built ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`,
      );
    }
    if (mutated.native_mut_string_len(built) !== utf8Length(expected)) {
      throw new Error(`${mode}: ${symbol}(${value}, ${suffix}) has the wrong UTF-8 byte length`);
    }
    if (mutated.native_mut_string_capacity(built) < utf8Length(expected)) {
      throw new Error(`${mode}: ${symbol}(${value}, ${suffix}) has insufficient capacity`);
    }
  }
}

NODE

for name in "" mut-; do
	if [ "$lane" = baseline ]; then
		# The v1 text optimizer is deleted: `js-opt=on` on the baseline lane no longer
		# rewrites anything, so the receipt is byte-frozen -- `on` must equal `off` exactly.
		cmp "$work/${name}off.js" "$work/${name}on.js"
		echo "native integer String lowering (${name:-immutable}, baseline lane): frozen, on == off"
		continue
	fi
	off_bytes=$(wc -c <"$work/${name}off.js" | tr -d ' ')
	on_bytes=$(wc -c <"$work/${name}on.js" | tr -d ' ')
	if ((on_bytes >= off_bytes)); then
		echo "native integer String lowering (jir lane) did not reduce output: $off_bytes -> $on_bytes" >&2
		exit 1
	fi
	echo "native integer String lowering (${name:-immutable}, jir lane): PASS ($off_bytes -> $on_bytes bytes)"
done
