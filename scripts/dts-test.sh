#!/usr/bin/env bash
#
# The `.d.ts` suite: what an emitted module looks like to TypeScript.
#
#   scripts/dts-test.sh                  # check every fixture
#   scripts/dts-test.sh 01_shapes
#   UPDATE_EXPECT=1 scripts/dts-test.sh  # rewrite the goldens from the output
#
# Each fixture in examples/dts-tests/ is a `#![no_std]` crate compiled with
# `-Cllvm-args=js-dts=on`, which makes the link step write a `.d.ts` beside the `.js`. It carries
# one golden:
#
#   NN.d.ts.expected   the declaration file, byte for byte
#
# and scripts/dts-check.mjs then asserts the properties a carelessly rewritten golden would break.
#
# WHY THERE IS NO `tsc` HERE
# --------------------------
# There is no TypeScript compiler anywhere in this tree: contract/vendor/node_modules holds babel,
# solid, dom-expressions, parse5, prettier and seroval, and no `typescript`. So the goldens are
# checked structurally instead, and what that cannot tell you is whether the text type checks.
# Vendoring `tsc` is a wave-6 ask and would turn the whole suite from "this is what we meant to
# write" into "this is valid TypeScript".

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

fixture_dir="$root/examples/dts-tests"
build_dir="$root/build/dtstest"
log_dir="$root/build/logs"

DTS_ARGS="js-names=readable js-comments=on js-modules=esm js-dts=on"

mkdir -p "$build_dir" "$log_dir"

# ---------------------------------------------------------------- prerequisites

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "dts-test.sh: backend build failed" >&2
		exit 1
	fi
fi

echo "==> building js-extern-macro for the host"
if ! (cd "$root" && cargo build -p js-extern-macro) >"$log_dir/dts-macro.log" 2>&1; then
	echo "dts-test.sh: could not build the proc macro" >&2
	cat "$log_dir/dts-macro.log" >&2
	exit 1
fi
case "$(uname -s)" in
Darwin) proc_macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) proc_macro="$root/target/debug/libjs_extern_macro.so" ;;
esac

echo "==> building view-abi through the backend"
abi_rlib="$build_dir/libview_abi.rlib"
if ! JS_EXTRA_ARGS="$DTS_ARGS" "$script_dir/compile_core.sh" "$root/view-abi/src/lib.rs" \
	"$abi_rlib" --crate-type rlib --crate-name view_abi >"$log_dir/dts-abi.log" 2>&1; then
	echo "dts-test.sh: could not build view-abi as an rlib" >&2
	cat "$log_dir/dts-abi.log" >&2
	exit 1
fi

# ------------------------------------------------------------------- selection

selected=${1:-}
if [ -n "$selected" ]; then
	name=$(basename -- "$selected")
	name=${name%.rs}
	if [ ! -f "$fixture_dir/$name.rs" ]; then
		echo "dts-test.sh: no such fixture: $name" >&2
		exit 2
	fi
	sources=$fixture_dir/$name.rs
else
	sources=$(ls "$fixture_dir"/*.rs 2>/dev/null | sort)
	if [ -z "$sources" ]; then
		echo "dts-test.sh: no fixtures found in $fixture_dir" >&2
		exit 1
	fi
fi

results=""
failures=0
updated=0
total=0

for src in $sources; do
	name=$(basename -- "${src%.rs}")
	out="$build_dir/$name.js"
	dts="$build_dir/$name.d.ts"
	expected="$fixture_dir/$name.d.ts.expected"
	total=$((total + 1))

	echo "==> $name"

	rm -f "$dts"
	if ! JS_EXTRA_ARGS="$DTS_ARGS" "$script_dir/compile_core.sh" "$src" "$out" \
		--extern view_abi="$abi_rlib" --extern js_extern_macro="$proc_macro" \
		>"$log_dir/dts-$name.compile.log" 2>&1; then
		echo "    compile failed:"
		sed 's/^/    /' "$log_dir/dts-$name.compile.log"
		results="$results$name FAIL compile-error"$'\n'
		failures=$((failures + 1))
		continue
	fi

	if [ ! -f "$dts" ]; then
		echo "    no .d.ts was written; is js-dts=on reaching the link step?"
		results="$results$name FAIL no-dts"$'\n'
		failures=$((failures + 1))
		continue
	fi

	if [ "${UPDATE_EXPECT:-0}" = "1" ]; then
		if ! cmp -s "$dts" "$expected" 2>/dev/null; then
			cp "$dts" "$expected"
			echo "    updated"
			results="$results$name UPDATED d.ts"$'\n'
			updated=$((updated + 1))
		else
			echo "    ok (unchanged)"
			results="$results$name PASS unchanged"$'\n'
		fi
	elif [ ! -f "$expected" ]; then
		echo "    missing expectation; run UPDATE_EXPECT=1 scripts/dts-test.sh $name"
		results="$results$name FAIL no-expectation"$'\n'
		failures=$((failures + 1))
		continue
	elif ! diff -u "$expected" "$dts" >"$log_dir/dts-$name.diff"; then
		echo "    the declaration file changed (- expected, + actual):"
		sed 's/^/    /' "$log_dir/dts-$name.diff"
		echo "    if this is an improvement: UPDATE_EXPECT=1 scripts/dts-test.sh $name"
		results="$results$name FAIL dts-mismatch"$'\n'
		failures=$((failures + 1))
		continue
	else
		echo "    ok (golden matches)"
		results="$results$name PASS golden"$'\n'
	fi
done

# ------------------------------------------------------ what the declarations say

echo "==> structure"
if node "$script_dir/dts-check.mjs" >"$log_dir/dts-structure.log" 2>&1; then
	checks=$(grep -c '^ok  ' "$log_dir/dts-structure.log")
	echo "    ok ($checks checks over the declarations)"
	results="$results structure PASS $checks-checks"$'\n'
	total=$((total + 1))
else
	echo "    structure failed:"
	grep -E '^FAIL|failed' "$log_dir/dts-structure.log" | sed 's/^/    /'
	results="$results structure FAIL see-$log_dir/dts-structure.log"$'\n'
	failures=$((failures + 1))
	total=$((total + 1))
fi

echo
echo "================================================================"
printf '%-24s %-8s %s\n' "FIXTURE" "RESULT" "DETAIL"
echo "----------------------------------------------------------------"
printf '%s' "$results" | while read -r name result detail; do
	printf '%-24s %-8s %s\n' "$name" "$result" "$detail"
done
echo "----------------------------------------------------------------"
if [ "$updated" -ne 0 ]; then
	printf '%d/%d passed, %d rebaselined\n' "$((total - failures - updated))" "$total" "$updated"
else
	printf '%d/%d passed\n' "$((total - failures))" "$total"
fi
echo "================================================================"

if [ "$failures" -ne 0 ]; then
	exit 1
fi
