#!/usr/bin/env bash
#
# Builds the backend, compiles every test to JavaScript, runs it under node and
# diffs stdout against the matching .expected file.
#
#   scripts/test.sh              # run every test
#   scripts/test.sh 03_structs   # run one test (name, file name or path)
#
# Two suites run, in this order:
#
#   examples/tests/       `#![no_core]` crates against examples/mini_core.rs,
#                         compiled with scripts/compile.sh (single crate,
#                         `--emit=obj`).
#   examples/core-tests/  `#![no_std]` crates against the REAL `core`, compiled
#                         with scripts/compile_core.sh against the sysroot that
#                         scripts/build_sysroot.sh builds. Skipped with a notice
#                         if that sysroot is absent, so this script still works
#                         on a fresh checkout — run ./scripts/build_sysroot.sh
#                         once to enable them.
#
# A core test may also name `mires_js`, the host collections crate, which this
# script builds through the backend into an rlib and offers to every one of them
# with `--extern`. A test that does not name it links nothing.
#
# Set SKIP_BUILD=1 to reuse an already-built backend dylib.
#
# Per test companion files, all optional except the first:
#
#   NN.expected     exact expected stdout (required for a normal test)
#   NN.maxbytes     a byte budget for the emitted .js; catches size regressions
#                   from a lost dead-code elimination. Defaults to
#                   $DEFAULT_MAXBYTES below. NOT enforced when $JS_EXTRA_ARGS
#                   asks for `js-line-comments`, `js-source-map` or `js-minify`:
#                   the first two add output whose whole point is to be there
#                   and the third removes output on purpose, the budgets are
#                   baselined against the default mode, and a budget that has to
#                   be moved to fit another mode stops catching the regressions
#                   it exists for. Sizes are still reported.
#   NN.absent       one grep -F pattern per line; none may appear in the .js.
#                   This is how a "the dead item is really gone" test is written.
#   NN.expect_fail  marks a test that MUST fail to compile; every non-empty line
#                   is a substring the compiler output has to contain. Such a
#                   test is never run under node and has no .expected.
#   NN.expect_abort marks a test that MUST compile, print its .expected and then
#                   exit non-zero: a panic that reaches the `#[panic_handler]`.
#                   A clean exit is a failure. (core suite only)
#
# With `JS_EXTRA_ARGS='js-modules=esm'` every program is an ES module, so it runs
# through scripts/run-esm.sh instead of the concatenation above: the shim is
# imported rather than prepended, and `rust_entry` is imported by name rather than
# called out of the surrounding scope. Nothing else about the run changes.
#
# $PENDING below names the tests that are known to fail because the feature they
# cover has not landed yet. They are reported as PENDING and do not fail the
# suite. Delete a name from the list in the same change that makes it pass.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

tests_dir="$root/examples/tests"
core_dir="$root/examples/core-tests"
sysroot="$root/build/sysroot"
build_dir="$root/build"
# Logs and captured output live beside, not inside, the directory rustc emits
# into, so nothing here can be mistaken for a compiler artifact.
log_dir="$build_dir/logs"
shim="$root/runtime/shim.js"
entry="$script_dir/entry.js"

# Nothing is pending: every test in both suites is expected to pass.
PENDING=""

# How a compiled program is run, which is the one thing `js-modules` decides here.
case " ${JS_EXTRA_ARGS:-} " in
*js-modules=esm*) esm=1 ;;
*) esm=0 ;;
esac

# emit_args <args>: the emit-affecting subset of a $JS_EXTRA_ARGS string.
#
# An option is emit-affecting when it can change the text of an item two crates
# might both codegen, which is the whole of what a sysroot and the crate on top of
# it have to agree on. The module options are not: everything they add is
# synthesized by the link step (scripts/module-test.sh holds that), so switching
# modes must not cost a sysroot rebuild.
emit_args() {
	local out=""
	local arg
	for arg in $1; do
		case "$arg" in
		js-modules=* | js-shim-module=* | js-dom-module=*) continue ;;
		esac
		out="$out${out:+ }$arg"
	done
	printf '%s' "$out"
}

# run_program <program.js> <name> <stdout> <stderr>: runs a compiled program.
run_program() {
	if [ "$esm" -eq 1 ]; then
		"$script_dir/run-esm.sh" "$1" "$2" >"$3" 2>"$4"
	else
		cat "$shim" "$1" "$entry" | node - >"$3" 2>"$4"
	fi
}

# The byte budgets describe the default output. A mode that deliberately emits
# more is exempt from them; see the .maxbytes note in the header.
enforce_budgets=1
case " ${JS_EXTRA_ARGS:-} " in
*js-line-comments* | *js-source-map* | *js-minify* | *js-structure=trampoline* | *js-queue=off*) enforce_budgets=0 ;;
esac
if [ "$enforce_budgets" -eq 0 ]; then
	echo "note: byte budgets are not enforced under JS_EXTRA_ARGS='${JS_EXTRA_ARGS:-}'"
fi

# Generous enough that no current test is near it, tight enough that losing
# dead-code elimination (which alone costs ~60 KB per test) fails the suite.
DEFAULT_MAXBYTES=24576

# A core test carries whatever slice of the real `core` it reaches, so the
# default is looser; each test pins its own budget in NN.maxbytes anyway.
DEFAULT_CORE_MAXBYTES=131072

mkdir -p "$build_dir" "$log_dir"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh"; then
		echo "test.sh: backend build failed" >&2
		exit 1
	fi
	echo
fi

is_pending() {
	case " $PENDING " in
	*" $1 "*) return 0 ;;
	*) return 1 ;;
	esac
}

# Collect the tests to run. `prelude.rs` is the core suite's shared header, not a
# test of its own.
all_core_sources() {
	ls "$core_dir"/*.rs 2>/dev/null | grep -v '/prelude\.rs$' | sort
}

selected=${1:-}
sources=""
core_sources=""
if [ -n "$selected" ]; then
	name=$(basename -- "$selected")
	name=${name%.rs}
	if [ -f "$tests_dir/$name.rs" ]; then
		sources=$tests_dir/$name.rs
	elif [ -f "$core_dir/$name.rs" ] && [ "$name" != prelude ]; then
		core_sources=$core_dir/$name.rs
	else
		echo "test.sh: no such test: $name" >&2
		echo "test.sh: available:" >&2
		{ ls "$tests_dir"/*.rs 2>/dev/null; all_core_sources; } | while read -r f; do
			echo "  $(basename -- "${f%.rs}")" >&2
		done
		exit 2
	fi
else
	sources=$(ls "$tests_dir"/*.rs 2>/dev/null | sort)
	core_sources=$(all_core_sources)
	if [ -z "$sources" ]; then
		echo "test.sh: no tests found in $tests_dir" >&2
		exit 1
	fi
fi

results=""
failures=0
pending=0
total=0

# record <name> <result> <detail>: appends a summary row and counts it. A
# failure of a pending test is downgraded rather than counted.
record() {
	local result=$2
	if [ "$result" = FAIL ] && is_pending "$1"; then
		result=PENDING
		pending=$((pending + 1))
	elif [ "$result" = FAIL ]; then
		failures=$((failures + 1))
	fi
	results="$results$1 $result $3"$'\n'
}

for src in $sources; do
	name=$(basename -- "${src%.rs}")
	out="$build_dir/$name.js"
	expected="$tests_dir/$name.expected"
	expect_fail="$tests_dir/$name.expect_fail"
	maxbytes_file="$tests_dir/$name.maxbytes"
	absent="$tests_dir/$name.absent"
	actual="$log_dir/$name.actual"
	compile_log="$log_dir/$name.compile.log"
	run_log="$log_dir/$name.run.log"
	diff_log="$log_dir/$name.diff"
	total=$((total + 1))

	echo "==> $name"

	# ---------------------------------------------------------------------
	# Expected-failure tests: the compiler has to reject this, and say why.
	# ---------------------------------------------------------------------
	if [ -f "$expect_fail" ]; then
		if "$script_dir/compile.sh" "$src" "$out" >"$compile_log" 2>&1; then
			echo "    compiled, but was expected to fail"
			record "$name" FAIL unexpected-success
			continue
		fi

		missing=""
		while IFS= read -r pattern; do
			[ -z "$pattern" ] && continue
			if ! grep -qF -- "$pattern" "$compile_log"; then
				missing="$missing$pattern"$'\n'
			fi
		done <"$expect_fail"

		if [ -n "$missing" ]; then
			echo "    failed as expected, but the message is missing:"
			printf '%s' "$missing" | sed 's/^/      /'
			echo "    actual output:"
			sed 's/^/      /' "$compile_log"
			record "$name" FAIL wrong-message
		else
			echo "    ok (failed as expected)"
			record "$name" PASS expected-failure
		fi
		continue
	fi

	# ---------------------------------------------------------------------
	# Normal tests: compile, run, diff, then check the size and absence rules.
	# ---------------------------------------------------------------------
	if [ ! -f "$expected" ]; then
		echo "    missing expectation: $expected"
		record "$name" FAIL no-.expected-file
		continue
	fi

	if ! "$script_dir/compile.sh" "$src" "$out" >"$compile_log" 2>&1; then
		echo "    compile failed:"
		sed 's/^/    /' "$compile_log"
		record "$name" FAIL compile-error
		continue
	fi

	if ! run_program "$out" "$name" "$actual" "$run_log"; then
		echo "    runtime error:"
		sed 's/^/    /' "$run_log"
		record "$name" FAIL runtime-error
		continue
	fi

	if ! diff -u "$expected" "$actual" >"$diff_log"; then
		echo "    output mismatch (- expected, + actual):"
		sed 's/^/    /' "$diff_log"
		record "$name" FAIL output-mismatch
		continue
	fi

	size=$(wc -c <"$out" | tr -d ' ')
	budget=$DEFAULT_MAXBYTES
	if [ -f "$maxbytes_file" ]; then
		budget=$(tr -dc '0-9' <"$maxbytes_file")
	fi
	if [ "$enforce_budgets" -eq 1 ] && [ "$size" -gt "$budget" ]; then
		echo "    over budget: $size bytes > $budget (see $maxbytes_file)"
		record "$name" FAIL "over-budget-$size"
		continue
	fi

	if [ -f "$absent" ]; then
		present=""
		while IFS= read -r pattern; do
			[ -z "$pattern" ] && continue
			if grep -qF -- "$pattern" "$out"; then
				present="$present$pattern"$'\n'
			fi
		done <"$absent"
		if [ -n "$present" ]; then
			echo "    these should have been eliminated, but are in the output:"
			printf '%s' "$present" | sed 's/^/      /'
			record "$name" FAIL not-eliminated
			continue
		fi
	fi

	echo "    ok ($size bytes)"
	record "$name" PASS "$size bytes"
done

# -------------------------------------------------------------------------
# The core suite: the same crates, but against the real `core`.
# -------------------------------------------------------------------------

if [ -n "$core_sources" ] && [ ! -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
	echo
	echo "==> core suite skipped: no sysroot at $sysroot"
	echo "    build one with ./scripts/build_sysroot.sh (about a minute), then re-run"
	core_sources=""
fi

# The sysroot has to have been built with the same emit-affecting $JS_EXTRA_ARGS
# as the crates linking against it. An option that changes the JavaScript an item
# prints — js-minify, js-structure, js-queue — makes a generic instantiation that
# both `core` and the test crate codegen come out spelled two ways, and link.rs
# reports that (correctly) as two different definitions of one item. Rebuilding
# is the fix; build_sysroot.sh is idempotent and notices the change itself.
if [ -n "$core_sources" ]; then
	sysroot_args=""
	[ -f "$sysroot/.js-args" ] && sysroot_args=$(cat "$sysroot/.js-args")
	want_args=$(emit_args "${JS_EXTRA_ARGS:-}")
	if [ "$(emit_args "$sysroot_args")" != "$want_args" ]; then
		echo
		echo "==> the sysroot was built with JS_EXTRA_ARGS='$sysroot_args', this run wants"
		echo "    '$want_args' — rebuilding it (about a minute)"
		if JS_EXTRA_ARGS="$want_args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" \
			>"$log_dir/build_sysroot.log" 2>&1; then
			echo "    ok"
		else
			echo "    sysroot rebuild failed, see $log_dir/build_sysroot.log"
			echo "==> core suite skipped"
			core_sources=""
		fi
	fi
fi

core_build_dir="$build_dir/core"
mkdir -p "$core_build_dir"

# `mires-js` is compiled by THIS backend into an rlib and offered to every core test, exactly as
# scripts/async-test.sh does with `view-abi`. A test that never names it links nothing: `--extern`
# only puts the crate on the search path, so the budgets of the other tests do not move.
#
# It is built with the emit-affecting options the sysroot was built with, for the reason the
# sysroot check above gives: an option that changes the JavaScript an item prints would make a
# generic instantiation that both crates codegen come out spelled two ways, and link.rs reports
# that as two definitions of one item.
mires_js_rlib="$core_build_dir/libmires_js.rlib"
if [ -n "$core_sources" ]; then
	echo
	echo "==> building mires-js through the backend"
	if JS_EXTRA_ARGS="$(emit_args "${JS_EXTRA_ARGS:-}")" "$script_dir/compile_core.sh" \
		"$root/mires-js/src/lib.rs" "$mires_js_rlib" \
		--crate-type rlib --crate-name mires_js >"$log_dir/mires-js.log" 2>&1; then
		echo "    ok"
	else
		echo "    failed, see $log_dir/mires-js.log"
		sed 's/^/    /' "$log_dir/mires-js.log"
		echo "==> core suite skipped"
		core_sources=""
	fi
fi
core_externs=(--extern "mires_js=$mires_js_rlib")

for src in $core_sources; do
	name=$(basename -- "${src%.rs}")
	out="$core_build_dir/$name.js"
	expected="$core_dir/$name.expected"
	expect_fail="$core_dir/$name.expect_fail"
	expect_abort="$core_dir/$name.expect_abort"
	maxbytes_file="$core_dir/$name.maxbytes"
	absent="$core_dir/$name.absent"
	actual="$log_dir/$name.actual"
	compile_log="$log_dir/$name.compile.log"
	run_log="$log_dir/$name.run.log"
	diff_log="$log_dir/$name.diff"
	total=$((total + 1))

	echo "==> $name (core)"

	# ---------------------------------------------------------------------
	# Expected-failure tests, same convention as the mini_core suite above.
	# Against real `core` this is how the *quality* of a rejection is pinned:
	# the zombie's message, and the `required by` chain back to the user's own
	# code that made it reachable.
	# ---------------------------------------------------------------------
	if [ -f "$expect_fail" ]; then
		if "$script_dir/compile_core.sh" "$src" "$out" "${core_externs[@]}" \
			>"$compile_log" 2>&1; then
			echo "    compiled, but was expected to fail"
			record "core/$name" FAIL unexpected-success
			continue
		fi

		missing=""
		while IFS= read -r pattern; do
			[ -z "$pattern" ] && continue
			if ! grep -qF -- "$pattern" "$compile_log"; then
				missing="$missing$pattern"$'\n'
			fi
		done <"$expect_fail"

		if [ -n "$missing" ]; then
			echo "    failed as expected, but the message is missing:"
			printf '%s' "$missing" | sed 's/^/      /'
			echo "    actual output:"
			sed 's/^/      /' "$compile_log"
			record "core/$name" FAIL wrong-message
		else
			echo "    ok (failed as expected)"
			record "core/$name" PASS expected-failure
		fi
		continue
	fi

	if [ ! -f "$expected" ]; then
		echo "    missing expectation: $expected"
		record "core/$name" FAIL no-.expected-file
		continue
	fi

	if ! "$script_dir/compile_core.sh" "$src" "$out" "${core_externs[@]}" \
		>"$compile_log" 2>&1; then
		echo "    compile failed:"
		sed 's/^/    /' "$compile_log"
		record "core/$name" FAIL compile-error
		continue
	fi

	run_program "$out" "core-$name" "$actual" "$run_log"
	status=$?

	# An `.expect_abort` test panics on purpose: node must report a failure, and
	# the output printed *before* the abort still has to match.
	if [ -f "$expect_abort" ]; then
		if [ "$status" -eq 0 ]; then
			echo "    ran to completion, but was expected to abort"
			record "core/$name" FAIL unexpected-success
			continue
		fi
	elif [ "$status" -ne 0 ]; then
		echo "    runtime error:"
		sed 's/^/    /' "$run_log"
		record "core/$name" FAIL runtime-error
		continue
	fi

	if ! diff -u "$expected" "$actual" >"$diff_log"; then
		echo "    output mismatch (- expected, + actual):"
		sed 's/^/    /' "$diff_log"
		record "core/$name" FAIL output-mismatch
		continue
	fi

	size=$(wc -c <"$out" | tr -d ' ')
	budget=$DEFAULT_CORE_MAXBYTES
	if [ -f "$maxbytes_file" ]; then
		budget=$(tr -dc '0-9' <"$maxbytes_file")
	fi
	if [ "$enforce_budgets" -eq 1 ] && [ "$size" -gt "$budget" ]; then
		echo "    over budget: $size bytes > $budget (see $maxbytes_file)"
		record "core/$name" FAIL "over-budget-$size"
		continue
	fi

	if [ -f "$absent" ]; then
		present=""
		while IFS= read -r pattern; do
			[ -z "$pattern" ] && continue
			if grep -qF -- "$pattern" "$out"; then
				present="$present$pattern"$'\n'
			fi
		done <"$absent"
		if [ -n "$present" ]; then
			echo "    these should have been eliminated, but are in the output:"
			printf '%s' "$present" | sed 's/^/      /'
			record "core/$name" FAIL not-eliminated
			continue
		fi
	fi

	if [ -f "$expect_abort" ]; then
		echo "    ok ($size bytes, aborted as expected)"
		record "core/$name" PASS "$size bytes, aborted"
	else
		echo "    ok ($size bytes)"
		record "core/$name" PASS "$size bytes"
	fi
done

echo
echo "================================================================"
printf '%-22s %-8s %s\n' "TEST" "RESULT" "DETAIL"
echo "----------------------------------------------------------------"
printf '%s' "$results" | while read -r name result detail; do
	printf '%-22s %-8s %s\n' "$name" "$result" "$detail"
done
echo "----------------------------------------------------------------"
printf '%d/%d passed' "$((total - failures - pending))" "$((total - pending))"
if [ "$pending" -ne 0 ]; then
	printf ' (%d pending)' "$pending"
fi
printf '\n'
echo "================================================================"

if [ "$failures" -ne 0 ]; then
	exit 1
fi
