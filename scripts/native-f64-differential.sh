#!/usr/bin/env bash
# Differentially checks native f64 formatting against Rust's Display implementation.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
oracle="$root/examples/native-tests/f64_to_string_oracle.rs"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"

rustc "+$channel" --edition=2024 -O "$oracle" -o "$work/oracle"

check="$work/check.js"
cat >"$check" <<'NODE'
const crypto = require("node:crypto");
const readline = require("node:readline");

require(process.argv[2]);

const bytes = new ArrayBuffer(8);
const view = new DataView(bytes);
const hash = crypto.createHash("sha256");
const started = process.hrtime.bigint();
let count = 0;

const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
lines.on("line", (line) => {
  const tab = line.indexOf("\t");
  if (tab < 0) throw new Error(`malformed oracle line ${count + 1}`);

  const hex = line.slice(0, tab);
  const expected = line.slice(tab + 1);
  view.setBigUint64(0, BigInt(`0x${hex}`), false);
  const actual = globalThis.__rt.native_f64_to_string(view.getFloat64(0, false)).text;
  count += 1;
  if (actual !== expected) {
    throw new Error(
      `f64 mismatch at case ${count}, bits 0x${hex}: Rust ${JSON.stringify(expected)}, JS ${JSON.stringify(actual)}`,
    );
  }
  hash.update(line);
  hash.update("\n");
});
lines.on("close", () => {
  if (count < 1_000_000) throw new Error(`oracle produced only ${count} cases`);
  const elapsed = Number(process.hrtime.bigint() - started) / 1e9;
  console.log(
    `native f64 differential: PASS (${count} cases, ${elapsed.toFixed(3)}s, sha256 ${hash.digest("hex")})`,
  );
});
NODE

"$work/oracle" | node "$check" "$root/runtime/shim.js"
