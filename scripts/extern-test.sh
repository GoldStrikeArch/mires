#!/usr/bin/env bash
#
# The `#[js_extern]` suite: declared JavaScript interfaces compiled to ES modules and driven
# against the contract's fake chart library.
#
#   scripts/extern-test.sh                  # check every fixture
#   scripts/extern-test.sh 01_chart
#   UPDATE_EXPECT=1 scripts/extern-test.sh  # rewrite the goldens from the output
#
# Each fixture in examples/extern-tests/ is a `#![no_std]` crate that declares an interface with
# `js_extern_macro::js_extern` and exports one entry point per contract vector. It carries one
# golden:
#
#   NN.js.expected     the emitted JavaScript, byte for byte
#
# and the meaning of that emission is checked separately by two drivers. scripts/extern-check.mjs
# runs the compiled module against contract/fixtures/js-extern/chart-lib.mjs and compares the
# recorded call trace with contract/fixtures/js-extern/vectors.json; scripts/globals-check.mjs does
# the same for declarations rooted at the global scope, against a recorder it installs as those
# globals. A golden pins WHAT was emitted; a driver pins that it does what the reference emission
# does, which is the thing a descriptor is about.
#
# HOW A FIXTURE IS COMPILED
# -------------------------
# `js_extern` is a proc macro, so `js-extern-macro` is built by plain cargo for the HOST: a proc
# macro runs in the compiler's own process and rustc loads the host dylib whatever `--target` says.
# The fixture is then compiled in one pass, macro and all: the expansion is ordinary Rust functions
# carrying a `link_section`, which needs no rewriting between expanding and compiling.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

fixture_dir="$root/examples/extern-tests"
build_dir="$root/build/externtest"
log_dir="$root/build/logs"

EXTERN_ARGS="js-names=readable js-comments=on js-modules=esm"

mkdir -p "$build_dir" "$log_dir"

# ---------------------------------------------------------------- prerequisites

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "extern-test.sh: backend build failed" >&2
		exit 1
	fi
fi

echo "==> building js-extern-macro for the host"
if ! (cd "$root" && cargo build -p js-extern-macro) >"$log_dir/extern-macro.log" 2>&1; then
	echo "extern-test.sh: could not build the proc macro" >&2
	cat "$log_dir/extern-macro.log" >&2
	exit 1
fi
case "$(uname -s)" in
Darwin) proc_macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) proc_macro="$root/target/debug/libjs_extern_macro.so" ;;
esac

# The run directory: a module package and the ES module shim. The chart library itself is written
# by scripts/extern-check.mjs, which is what resolves the declared specifier.
printf '{ "type": "module" }\n' >"$build_dir/package.json"
if ! node "$script_dir/make-esm-shim.mjs" "$build_dir/shim.js"; then
	echo "extern-test.sh: could not derive the ES module shim" >&2
	exit 1
fi

# ------------------------------------------------------------------- selection

selected=${1:-}
if [ -n "$selected" ]; then
	name=$(basename -- "$selected")
	name=${name%.rs}
	if [ ! -f "$fixture_dir/$name.rs" ]; then
		echo "extern-test.sh: no such fixture: $name" >&2
		exit 2
	fi
	sources=$fixture_dir/$name.rs
else
	sources=$(ls "$fixture_dir"/*.rs 2>/dev/null | sort)
	if [ -z "$sources" ]; then
		echo "extern-test.sh: no fixtures found in $fixture_dir" >&2
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
	expected_js="$fixture_dir/$name.js.expected"
	total=$((total + 1))

	echo "==> $name"

	if ! JS_EXTRA_ARGS="$EXTERN_ARGS" "$script_dir/compile_core.sh" "$src" "$out" \
		--extern js_extern_macro="$proc_macro" >"$log_dir/extern-$name.compile.log" 2>&1; then
		echo "    compile failed:"
		sed 's/^/    /' "$log_dir/extern-$name.compile.log"
		results="$results$name FAIL compile-error"$'\n'
		failures=$((failures + 1))
		continue
	fi

	if [ "${UPDATE_EXPECT:-0}" = "1" ]; then
		if ! cmp -s "$out" "$expected_js" 2>/dev/null; then
			cp "$out" "$expected_js"
			echo "    updated (js)"
			results="$results$name UPDATED js"$'\n'
			updated=$((updated + 1))
		else
			echo "    ok (unchanged)"
			results="$results$name PASS unchanged"$'\n'
		fi
	elif [ ! -f "$expected_js" ]; then
		echo "    missing expectation; run UPDATE_EXPECT=1 scripts/extern-test.sh $name"
		results="$results$name FAIL no-expectation"$'\n'
		failures=$((failures + 1))
		continue
	elif ! diff -u "$expected_js" "$out" >"$log_dir/extern-$name.js.diff"; then
		echo "    emitted JavaScript changed (- expected, + actual):"
		sed 's/^/    /' "$log_dir/extern-$name.js.diff"
		echo "    if this is an improvement: UPDATE_EXPECT=1 scripts/extern-test.sh $name"
		results="$results$name FAIL js-mismatch"$'\n'
		failures=$((failures + 1))
		continue
	else
		echo "    ok (golden matches)"
		results="$results$name PASS golden"$'\n'
	fi
done

# ------------------------------------------------------ the vectors themselves

echo "==> contract vectors"
if node "$script_dir/extern-check.mjs" >"$log_dir/extern-vectors.log" 2>&1; then
	checks=$(grep -c '^ok  ' "$log_dir/extern-vectors.log")
	echo "    ok ($checks checks against contract/fixtures/js-extern/vectors.json)"
	results="$results vectors PASS $checks-checks"$'\n'
	total=$((total + 1))
else
	echo "    vectors failed:"
	grep -E '^FAIL|failed' "$log_dir/extern-vectors.log" | sed 's/^/    /'
	results="$results vectors FAIL see-$log_dir/extern-vectors.log"$'\n'
	failures=$((failures + 1))
	total=$((total + 1))
fi

echo "==> globals"
if node "$script_dir/globals-check.mjs" >"$log_dir/extern-globals.log" 2>&1; then
	checks=$(grep -c '^ok  ' "$log_dir/extern-globals.log")
	echo "    ok ($checks checks against a recorder installed as the globals)"
	results="$results globals PASS $checks-checks"$'\n'
	total=$((total + 1))
else
	echo "    globals failed:"
	grep -E '^FAIL|failed' "$log_dir/extern-globals.log" | sed 's/^/    /'
	results="$results globals FAIL see-$log_dir/extern-globals.log"$'\n'
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
