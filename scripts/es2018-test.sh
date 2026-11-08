#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
parser_root=$script_dir/es2018-parser
parser=${ES2018_PARSER:-$parser_root/node_modules/acorn/dist/acorn.mjs}
work=$root/build/es2018

if [ ! -f "$parser" ]; then
	echo "es2018-test.sh: pinned Acorn parser is missing: $parser" >&2
	echo "es2018-test.sh: install it with: npm ci --prefix $parser_root" >&2
	exit 1
fi

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	"$script_dir/build.sh" >/dev/null
fi

rm -rf "$work"
mkdir -p "$work"

script=$work/bigint-script.js
module=$work/bigint-module.js
compact=$work/bigint-compact.js
chunks=$work/chunks.js

JS_EXTRA_ARGS='js-ecma-target=2018' "$script_dir/compile.sh" \
	"$root/examples/tests/12_bigint.rs" "$script"
JS_EXTRA_ARGS='js-ecma-target=2018 js-modules=esm js-source-map=on' "$script_dir/compile.sh" \
	"$root/examples/tests/12_bigint.rs" "$module"
JS_EXTRA_ARGS='js-ecma-target=2018 js-minify=on' "$script_dir/compile.sh" \
	"$root/examples/tests/12_bigint.rs" "$compact"
JS_EXTRA_ARGS='js-ecma-target=2018 js-modules=esm js-chunk=a:__island_a js-chunk=b:__island_b js-chunk-shared=shared' \
	"$script_dir/compile.sh" "$root/examples/emit/19_chunks.rs" "$chunks"

cat "$root/runtime/shim.js" "$script" "$script_dir/entry.js" | node - >"$work/script.actual"
diff -u "$root/examples/tests/12_bigint.expected" "$work/script.actual"
cat "$root/runtime/shim.js" "$compact" "$script_dir/entry.js" | node - >"$work/compact.actual"
diff -u "$root/examples/tests/12_bigint.expected" "$work/compact.actual"
"$script_dir/run-esm.sh" "$module" es2018-bigint >"$work/module.actual"
diff -u "$root/examples/tests/12_bigint.expected" "$work/module.actual"
node "$script_dir/check-sourcemap.js" "$module" 12_bigint.rs >/dev/null
"$script_dir/run-chunks.sh" "$chunks" es2018-chunks a:__island_a b:__island_b >"$work/chunks.actual"
diff -u "$root/examples/emit/19_chunks.expected" "$work/chunks.actual"

node "$script_dir/es2018-gate.mjs" \
	--parser "$parser" \
	--root "$root" \
	--output "$work/receipt.json" \
	--script "$root/runtime/shim.js" \
	--script "$script" \
	--script "$compact" \
	--module "$module" \
	--module "$work/a.js" \
	--module "$work/b.js" \
	--module "$work/shared.js" \
	>"$work/gate.stdout.json"

if rg -n --pcre2 '(?<![A-Za-z0-9_$])(?:0[xX][0-9A-Fa-f]+|[0-9]+)n\b|\|\|=|&&=|\?\?=' \
	"$root/runtime/shim.js" "$script" "$compact" "$module" "$work/a.js" "$work/b.js" "$work/shared.js"; then
	echo 'es2018-test.sh: a forbidden post-ES2018 token survived' >&2
	exit 1
fi

jq -e '
  .schema == 2 and
  .passed == true and
  .parser.version == "8.15.0" and
  .target == "ES2018" and
  .strictRuntimeCompatibilityClaimed == false and
  .opaqueInputPolicy.rawJavaScript == "validated-by-pinned-final-artifact-parser" and
  .selectedPostEs2018SyntaxCapabilities == [] and
  .postEs2018Features == {} and
  .runtimeInventoryKind == "selected-global-identifier-counts" and
  .runtimeRequirements.BigInt > 0 and
  .runtimeRequirements.queueMicrotask > 0 and
  (.files | length) == 7 and
  all(.files[]; .es2018 == true and .observedMinimum <= 2018)
' "$work/receipt.json" >/dev/null

if node "$script_dir/es2018-gate.mjs" \
	--parser "$parser" \
	--root "$root" \
	--output "$work/rejected.json" \
	--script "$script_dir/fixtures/post-es2018.js" \
	>/dev/null; then
	echo 'es2018-test.sh: the parser gate accepted post-ES2018 opaque input' >&2
	exit 1
fi

jq -e '
  .passed == false and
  .files[0].es2018 == false and
  .files[0].observedMinimum == 2021 and
  .postEs2018Features.BigIntLiteral == 1 and
  .postEs2018Features["LogicalAssignment:||="] == 1 and
  .postEs2018Features["LogicalAssignment:??="] == 1 and
  .postEs2018Features.OptionalChaining == 1
' "$work/rejected.json" >/dev/null

printf 'ES2018 target checks passed (7 artifacts, BigInt inventoried as a runtime requirement)\n'
