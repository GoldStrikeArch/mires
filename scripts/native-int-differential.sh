#!/usr/bin/env bash
# Differentially checks native integer, bool, and char to_string against Rust's Display.
#
# The oracle is an ordinary `rustc -O` host binary. Its cases run through the emitted lowering
# itself, not through a runtime helper, so the comparison covers the exported wrapper, the value
# model, and the native body together. `off` runs the same cases through Rust's own formatting
# compiled to JavaScript, which is what pins the argument representation the `on` lane reads.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
oracle="$root/examples/native-tests/int_to_string_oracle.rs"
fixture="$root/examples/native-tests/int_to_string_differential.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"

rustc "+$channel" --edition=2024 -O "$oracle" -o "$work/oracle"

for mode in off on; do
	args="js-opt=$mode js-opt-passes=native-representations"
	JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >/dev/null
	JS_EXTRA_ARGS="$args js-opt-warnings=off" \
		"$script_dir/compile_core.sh" "$fixture" "$work/$mode.js" >/dev/null
done

check="$work/check.js"
cat >"$check" <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const readline = require("node:readline");
const vm = require("node:vm");

const root = process.argv[2];
// Rust's own formatting compiled to JavaScript is slower than the native lane by orders of
// magnitude, so it verifies a deterministic stride of the same cases rather than all of them.
const BASELINE_STRIDE = 37;

function load(file) {
  const context = vm.createContext({ TextDecoder, TextEncoder });
  vm.runInContext(fs.readFileSync(`${root}/runtime/shim.js`, "utf8"), context);
  vm.runInContext(fs.readFileSync(file, "utf8"), context);
  return context;
}

const native = load(process.argv[3]);
const baseline = load(process.argv[4]);

const big = new Set(["i64", "u64", "i128", "u128"]);
// A leading U+FEFF is an ordinary `char` case here, not an encoding marker to strip.
const decoder = new TextDecoder("utf-8", { ignoreBOM: true });
const hash = crypto.createHash("sha256");
const started = process.hrtime.bigint();
const lanes = new Map();
let count = 0;

function argument(kind, input) {
  if (big.has(kind)) return BigInt(input);
  if (kind === "bool") return input === "true";
  return Number(input);
}

function run(context, kind, value) {
  return context.native_int_string_as_str(context[`native_${kind}_string`](value));
}

const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on("line", (line) => {
  const fields = line.split("\t");
  if (fields.length !== 3) throw new Error(`malformed oracle line ${count + 1}`);
  const [kind, input, hex] = fields;
  const expected = decoder.decode(Uint8Array.from(Buffer.from(hex, "hex")));
  const value = argument(kind, input);

  count += 1;
  lanes.set(kind, (lanes.get(kind) ?? 0) + 1);
  const actual = run(native, kind, value);
  if (actual !== expected) {
    throw new Error(
      `${kind} mismatch at case ${count}, input ${input}: Rust ${JSON.stringify(expected)}, native ${JSON.stringify(actual)}`,
    );
  }
  if (count % BASELINE_STRIDE === 0) {
    const printed = run(baseline, kind, value);
    if (printed !== expected) {
      throw new Error(
        `${kind} baseline mismatch at case ${count}, input ${input}: Rust ${JSON.stringify(expected)}, baseline ${JSON.stringify(printed)}`,
      );
    }
  }
  hash.update(line);
  hash.update("\n");
});
lines.on("close", () => {
  if (count < 1_000_000) throw new Error(`oracle produced only ${count} cases`);
  for (const kind of [
    "i8", "u8", "i16", "u16", "i32", "u32", "i64", "u64",
    "i128", "u128", "isize", "usize", "bool", "char",
  ]) {
    if (!lanes.has(kind)) throw new Error(`oracle produced no ${kind} cases`);
  }
  const elapsed = Number(process.hrtime.bigint() - started) / 1e9;
  console.log(
    `native integer differential: PASS (${count} cases, ${elapsed.toFixed(3)}s, sha256 ${hash.digest("hex")})`,
  );
});
NODE

"$work/oracle" | node "$check" "$root" "$work/on.js" "$work/off.js"
