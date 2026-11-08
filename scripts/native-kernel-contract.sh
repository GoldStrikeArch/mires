#!/usr/bin/env bash
# Checks the first JS-native lowering contract without building the codegen backend or its sysroot.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
fixture="$root/examples/native-tests/01_language_kernel.rs"
native_js="$root/examples/native-tests/01_language_kernel.js"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

rustc --edition=2024 -O "$fixture" -o "$work/rust-oracle"
"$work/rust-oracle" >"$work/rust.out"
node --check "$native_js"
node "$native_js" >"$work/js.out"
diff -u "$work/rust.out" "$work/js.out"

bytes=$(wc -c <"$native_js" | tr -d ' ')
if [ "$bytes" -gt 900 ]; then
	echo "native-kernel-contract.sh: $bytes bytes exceeds the 900-byte budget" >&2
	exit 1
fi

for shape in 'const summary = { value: 2, accepted: 0 }' 'for (const op of ops)' 'switch (op.tag)' '(value) =>'; do
	grep -qF -- "$shape" "$native_js"
done

if grep -Eq '__rt|(^|[^[:alnum:]_])(buf|off)[[:space:]]*:|ArrayBuffer|DataView|BigInt|WeakMap' "$native_js"; then
	echo "native-kernel-contract.sh: native output contains a legacy heap or pointer helper" >&2
	exit 1
fi

echo "native language kernel: PASS ($bytes bytes)"
