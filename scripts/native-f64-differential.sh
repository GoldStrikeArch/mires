#!/usr/bin/env bash
# Differentially checks native f64 formatting against Rust's Display implementation: the shim's
# shortest writer itself, and every call shape the compiler routes to it in a `js-cleanup=on`
# build (examples/native-tests/301_f64_format_paths.rs), compiled with the settings the idiomatic
# FingerprintJS port ships with and proven to have taken the native route.
#
# Set SKIP_BUILD=1 to reuse an already-built compiler.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# `301_f64_format_paths_js.rs` exercises the `js-cleanup=on` format-kernel route; default
# `JSC_PANIC` to `abort` (preserving any pre-existing `build/sysroot` symlink on exit) so
# `sysroot-cache.sh` does not override `js-profile=release-balanced` with `js-cleanup=off`.
export JSC_PANIC="${JSC_PANIC:-abort}"
prev_sysroot_link=""
if [ -L "$root/build/sysroot" ]; then
	prev_sysroot_link=$(readlink "$root/build/sysroot")
fi
oracle="$root/examples/native-tests/f64_to_string_oracle.rs"
fixture="$root/examples/native-tests/301_f64_format_paths_js.rs"
work=$(mktemp -d)
cleanup() {
	rm -rf -- "$work"
	if [ -n "$prev_sysroot_link" ]; then
		local tmp="$root/build/.sysroot-restore.$$"
		mkdir -p "$root/build"
		ln -s "$prev_sysroot_link" "$tmp"
		mv -Tf "$tmp" "$root/build/sysroot" 2>/dev/null || {
			rm -f "$root/build/sysroot" "$tmp"
			ln -s "$prev_sysroot_link" "$root/build/sysroot"
		}
	fi
}
trap cleanup EXIT

channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"

rustc "+$channel" --edition=2024 -O "$oracle" -o "$work/oracle"

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

# The idiomatic FingerprintJS port's settings: release-balanced (js-cleanup=on), the JIR lane,
# MIR inlining off.
args="js-profile=release-balanced js-lane=jir"
JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
JS_EXTRA_ARGS="$args js-opt-warnings=off js-opt-report=$work/paths.report.json" \
	"$script_dir/compile_core.sh" "$fixture" "$work/paths.js" >/dev/null

# Every comparison below would also pass on core's own float formatting, so first prove the call
# shapes took the native route: no emitted item is float formatting machinery (the report names
# items by Rust path; `js-minify` shortens them in the program text), and the native writer is
# what they call.
node - "$work/paths.report.json" "$work/paths.js" <<'NODE'
const fs = require("node:fs");
const [reportPath, programPath] = process.argv.slice(2);
const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
const machinery = report.items
  .map((item) => item.path ?? "")
  .filter((path) => /flt2dec|fmt::float|Display for f64|bignum|diy_float/.test(path));
if (machinery.length > 0) {
  throw new Error(`the f64 call shapes still link float formatting: ${machinery.join(", ")}`);
}
if (!fs.readFileSync(programPath, "utf8").includes("native_f64_to_string")) {
  throw new Error("the f64 call shapes do not call __rt.native_f64_to_string");
}
NODE

check="$work/check.js"
cat >"$check" <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const readline = require("node:readline");
const vm = require("node:vm");

require(process.argv[2]);

// The compiled call shapes, each sending its text to the host, where it is read back.
const context = vm.createContext({ TextDecoder, TextEncoder });
vm.runInContext(fs.readFileSync(process.argv[2], "utf8"), context);
vm.runInContext(fs.readFileSync(process.argv[3], "utf8"), context);
let sent;
context.__rt.js_log_str = (text) => {
  sent = text;
};
const shapes = ["f64_debug_line", "f64_text", "f64_reciprocal_text"];
function shown(shape, value) {
  sent = undefined;
  context[shape](value);
  return sent;
}

const bytes = new ArrayBuffer(8);
const view = new DataView(bytes);
const hash = crypto.createHash("sha256");
const started = process.hrtime.bigint();
let count = 0;

const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on("line", (line) => {
  const columns = line.split("\t");
  if (columns.length !== 2 + shapes.length) throw new Error(`malformed oracle line ${count + 1}`);

  const [hex, expected, ...shapeTexts] = columns;
  view.setBigUint64(0, BigInt(`0x${hex}`), false);
  const value = view.getFloat64(0, false);
  const actual = globalThis.__rt.native_f64_to_string(value).text;
  count += 1;
  if (actual !== expected) {
    throw new Error(
      `f64 mismatch at case ${count}, bits 0x${hex}: Rust ${JSON.stringify(expected)}, JS ${JSON.stringify(actual)}`,
    );
  }
  shapes.forEach((shape, index) => {
    const text = shown(shape, value);
    if (text !== shapeTexts[index]) {
      throw new Error(
        `${shape} mismatch at case ${count}, bits 0x${hex}: Rust ${JSON.stringify(shapeTexts[index])}, JS ${JSON.stringify(text)}`,
      );
    }
  });
  hash.update(line);
  hash.update("\n");
});
lines.on("close", () => {
  if (count < 1_000_000) throw new Error(`oracle produced only ${count} cases`);
  const elapsed = Number(process.hrtime.bigint() - started) / 1e9;
  console.log(
    `native f64 differential: PASS (${count} cases, writer + ${shapes.join(", ")} through the native route, ${elapsed.toFixed(3)}s, sha256 ${hash.digest("hex")})`,
  );
});
NODE

"$work/oracle" | node "$check" "$root/runtime/shim.js" "$work/paths.js"
