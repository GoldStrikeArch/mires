#!/usr/bin/env bash
#
# Runs a program compiled with `-Cllvm-args=js-modules=esm` under node.
#
#   scripts/run-esm.sh <program.js> [name]
#
# It is what `cat runtime/shim.js out.js scripts/entry.js | node -` is for a
# script mode program, and it differs in the two ways the module mode does:
#
#   * the shim is imported by the program rather than concatenated ahead of it,
#     so it has to be a file the program can resolve. build/esm/shim.js is the
#     ES module form of runtime/shim.js, derived by scripts/make-esm-shim.mjs;
#     the default `-Cllvm-args=js-shim-module=./shim.js` is what makes that name
#     the right one.
#   * `rust_entry` is an export, not a global, so the entry point imports it by
#     name instead of calling it out of the surrounding scope. That is also the
#     check that the trailing `export { .. }` clause names what it should: a
#     missing name is a SyntaxError before anything runs.
#
# Everything lands in build/esm/, whose package.json is what lets a `.js` file
# with imports be a module under node. `name` (default: the program's file name)
# keeps two suites with the same test names apart.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

if [ "$#" -lt 1 ]; then
	echo "usage: run-esm.sh <program.js> [name]" >&2
	exit 2
fi

program=$1
name=${2:-$(basename -- "${program%.js}")}

if [ ! -f "$program" ]; then
	echo "run-esm.sh: no such program: $program" >&2
	exit 1
fi

dir="$root/build/esm"
shim="$root/runtime/shim.js"
esm_shim="$dir/shim.js"

mkdir -p "$dir"

# `type: module` for the whole directory, so the compiled `.js` and the shim are
# modules without having to be renamed `.mjs`.
if [ ! -f "$dir/package.json" ]; then
	printf '{ "type": "module" }\n' >"$dir/package.json"
fi

# Derived once per change of the source, not once per test.
if [ ! -f "$esm_shim" ] || [ "$shim" -nt "$esm_shim" ]; then
	if ! node "$script_dir/make-esm-shim.mjs" "$esm_shim"; then
		echo "run-esm.sh: could not derive the ES module shim from $shim" >&2
		exit 1
	fi
fi

cp "$program" "$dir/$name.js"
cat >"$dir/$name.main.js" <<EOF
import { rust_entry } from "./$name.js";
rust_entry();
EOF

node "$dir/$name.main.js"
