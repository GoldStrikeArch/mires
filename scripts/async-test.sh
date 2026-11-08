#!/usr/bin/env bash
#
# The executor gate: Rust futures spawned, polled, resumed and cancelled inside a compiled program.
#
#   scripts/async-test.sh                    # every fixture, in each optimizer mode
#   scripts/async-test.sh 01_microtask       # one fixture
#   ASYNC_OPT_MODE=on scripts/async-test.sh  # one mode (or a space-separated list of them)
#
# Each fixture in examples/async-tests/ is a `#![no_std]` crate linked against `view-abi` and
# `view-async` and compiled to an ES module. scripts/async-check.mjs then installs the host
# interface the fixtures declare (examples/async-tests/prelude.rs, and the `js_bindings!` of 07 and
# 08), asserts the emission's shape, drives each entry point and asserts the sequence of steps the
# executor produced: deferral to a microtask, owner capture and re-entry, rejections as values,
# coalesced wakes, concurrent tasks, owner cancellation with exact drop order (07), and subscription
# exceptions on typed error paths (08).
#
# WHAT IS BUILT, AND HOW
# ----------------------
# The release configuration: `js-profile=release-balanced` (so `js-cleanup=on`, minified, ES2018)
# on the JIR lane, in each optimizer mode off/report/on. off and report drive the safe Rust
# executor and `on` the shared host driver (`__rt.host_spawn`); off and report must emit identical
# JavaScript, and no item of a fixture may fall back to the baseline lane. For each mode,
# `view-abi` and then `view-async` are compiled by THIS backend into rlibs with the fixtures' own
# options -- the DOM module specifier is emit affecting and the executor's crate emits imports of
# it -- and every fixture is compiled against them. `js-extern-macro` is a proc macro and is built
# by plain cargo for the host. Each mode's directory, build/asynctest/<mode>/, is also the run
# directory: a module package, the ES module shim, and the vendored DOM runtime, so owner capture,
# cleanup and re-entry execute in the same reactive graph as deployed applications. Everything is
# kept there, logs included. SKIP_BUILD=1 reuses the backend instead of running scripts/build.sh.
#
# WHAT IS NOT BUILT
# -----------------
# 06_procedure runs the Topcoat framework's own `#[procedure(serde)]` expansion unmodified; that is
# its whole point. It needs the framework's `topcoat_runtime_macro` proc macro and its `topcoat_dom`
# crate, and neither is part of this repository (docs/V2-RESTORATION.md: no framework crate or
# procedure runtime is a dependency). A hand-written expansion would answer a different question,
# so the fixture is reported SKIP rather than compiled.
#
# WHY THERE IS NO BYTE FOR BYTE GOLDEN HERE
# -----------------------------------------
# A fixture that spawns anything links `Vec`, `Rc` and the allocator, so its emission is mostly
# `core` around a few lines of executor. That `core` is pinned already, by the emit and core
# suites, and a golden of it here would rebaseline on every unrelated `core` change while hiding the
# lines that are actually about the executor. So the driver asserts the SHAPE instead -- that the
# scheduler is a microtask or the host driver, that the owner is captured through the DOM module,
# that the settled bridge is the shim's -- and then asserts what running it does.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"

fixture_dir="$root/examples/async-tests"
build_root="$root/build/asynctest"

# Fixtures that need crates this repository does not have; see the header.
unbuildable="06_procedure"

modes=${ASYNC_OPT_MODE:-off report on}
for mode in $modes; do
	case "$mode" in
	off | report | on) ;;
	*)
		echo "async-test.sh: ASYNC_OPT_MODE must be off, report, on, or a list of them" >&2
		exit 2
		;;
	esac
done

selected=${1:-}
if [ -n "$selected" ]; then
	names=$(basename -- "$selected")
	names=${names%.rs}
	if [ ! -f "$fixture_dir/$names.rs" ]; then
		echo "async-test.sh: no such fixture: $names" >&2
		exit 2
	fi
	case " $unbuildable " in
	*" $names "*)
		echo "async-test.sh: $names cannot be built in this repository; see the header" >&2
		exit 2
		;;
	esac
else
	names=$(cd "$fixture_dir" && ls [0-9]*.rs 2>/dev/null | sed 's/\.rs$//' | sort)
	if [ -z "$names" ]; then
		echo "async-test.sh: no fixtures found in $fixture_dir" >&2
		exit 1
	fi
fi

mkdir -p "$build_root"

# ---------------------------------------------------------------- prerequisites

if [ "${SKIP_BUILD:-0}" != 1 ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "async-test.sh: backend build failed" >&2
		exit 1
	fi
fi

echo "==> building js-extern-macro for the host"
if ! (cd "$root" && cargo build -p js-extern-macro --locked) >"$build_root/macro.log" 2>&1; then
	echo "async-test.sh: could not build js-extern-macro" >&2
	cat "$build_root/macro.log" >&2
	exit 1
fi
case "$(uname -s)" in
Darwin) extern_macro="$root/target/debug/libjs_extern_macro.dylib" ;;
*) extern_macro="$root/target/debug/libjs_extern_macro.so" ;;
esac

if ! node "$script_dir/make-esm-shim.mjs" "$build_root/shim.js"; then
	echo "async-test.sh: could not derive the ES module shim" >&2
	exit 1
fi

# ------------------------------------------------------------------- each mode

results=""
failures=0
total=0
# record <fixture> <mode> <PASS|FAIL|SKIP> <detail>
record() {
	results="$results$1 $2 $3 $4"$'\n'
	case "$3" in
	PASS) total=$((total + 1)) ;;
	FAIL)
		total=$((total + 1))
		failures=$((failures + 1))
		;;
	esac
}
for name in $names; do
	case " $unbuildable " in
	*" $name "*) record "$name" all SKIP "needs-topcoat_runtime_macro+topcoat_dom" ;;
	esac
done
# The compile log opens with the backend's per-body cleanup trace; a failure is at the end.
show_log() {
	grep -v '^cleanup-mir-import: ' "$1" | tail -n 20 | sed 's/^/      /'
}

for mode in $modes; do
	dir="$build_root/$mode"
	rm -rf "$dir"
	mkdir -p "$dir"
	args="js-profile=release-balanced js-opt=$mode js-lane=jir"
	module_args="$args js-comments=on js-modules=esm js-dom-module=./topcoat-dom.js"
	echo "==> $mode: $module_args"

	if ! JS_EXTRA_ARGS="$args" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" >"$dir/sysroot.log" 2>&1; then
		echo "    could not build the sysroot:"
		show_log "$dir/sysroot.log"
		record "(sysroot)" "$mode" FAIL "see-$dir/sysroot.log"
		continue
	fi

	abi_rlib="$dir/libview_abi.rlib"
	async_rlib="$dir/libview_async.rlib"
	if ! JS_EXTRA_ARGS="$module_args" "$script_dir/compile_core.sh" "$root/view-abi/src/lib.rs" \
		"$abi_rlib" --crate-type rlib --crate-name view_abi >"$dir/view_abi.log" 2>&1; then
		echo "    could not build view-abi as an rlib:"
		show_log "$dir/view_abi.log"
		record "(view-abi)" "$mode" FAIL "see-$dir/view_abi.log"
		continue
	fi
	if ! JS_EXTRA_ARGS="$module_args" "$script_dir/compile_core.sh" "$root/view-async/src/lib.rs" \
		"$async_rlib" --crate-type rlib --crate-name view_async --extern view_abi="$abi_rlib" \
		>"$dir/view_async.log" 2>&1; then
		echo "    could not build view-async as an rlib:"
		show_log "$dir/view_async.log"
		record "(view-async)" "$mode" FAIL "see-$dir/view_async.log"
		continue
	fi

	printf '{ "type": "module" }\n' >"$dir/package.json"
	cp "$build_root/shim.js" "$dir/shim.js"
	cp "$root/runtime/dom/dist/topcoat-dom.js" "$dir/topcoat-dom.js"

	compiled=()
	for name in $names; do
		case " $unbuildable " in
		*" $name "*) continue ;;
		esac
		out="$dir/$name.js"
		if ! JS_EXTRA_ARGS="$module_args" "$script_dir/compile_core.sh" "$fixture_dir/$name.rs" "$out" \
			--search "$dir" --extern view_abi="$abi_rlib" --extern view_async="$async_rlib" \
			--extern js_extern_macro="$extern_macro" >"$out.compile.log" 2>&1; then
			echo "    $name: compile failed:"
			show_log "$out.compile.log"
			record "$name" "$mode" FAIL compile-error
			continue
		fi
		fallbacks=$(node -e 'console.log(JSON.parse(require("fs").readFileSync(process.argv[1], "utf8")).fallback_count)' \
			"$out.lane.json" 2>/dev/null || echo "?")
		if [ "$fallbacks" != 0 ]; then
			echo "    $name: $fallbacks item(s) fell back to the baseline lane; see $out.lane.json"
			record "$name" "$mode" FAIL "jir-fallbacks-$fallbacks"
			continue
		fi
		compiled+=("$name")
	done

	if [ "${#compiled[@]}" -eq 0 ]; then
		continue
	fi
	ASYNC_OPT_MODE="$mode" node "$script_dir/async-check.mjs" "$dir" "${compiled[@]}" \
		>"$dir/behaviour.log" 2>&1
	status=$?
	for name in "${compiled[@]}"; do
		read -r ok failed < <(awk -v name="$name" '$1 == "summary" && $2 == name { print $3, $4 }' \
			"$dir/behaviour.log")
		if [ -z "${ok:-}" ]; then
			record "$name" "$mode" FAIL "no-result:see-$dir/behaviour.log"
		elif [ "$failed" -ne 0 ]; then
			record "$name" "$mode" FAIL "$failed/$((ok + failed))-checks-failed"
		elif [ "$ok" -eq 0 ]; then
			record "$name" "$mode" FAIL no-checks
		else
			record "$name" "$mode" PASS "$ok-checks"
		fi
		ok=""
	done
	if [ "$status" -ne 0 ]; then
		echo "    behaviour failed (exit $status):"
		grep -E '^FAIL|Error' "$dir/behaviour.log" | head -n 20 | sed 's/^/      /'
		case "$results" in
		*" $mode FAIL "*) ;;
		*) record "(driver)" "$mode" FAIL "exit-$status:see-$dir/behaviour.log" ;;
		esac
	fi
	echo "    $mode: ${#compiled[@]} fixture(s) compiled and run; see $dir/behaviour.log"
done

# off and report differ only in what the optimizer reports, so they must emit the same program.
if [[ " $modes " == *" off "* && " $modes " == *" report "* ]]; then
	for name in $names; do
		[ -f "$build_root/off/$name.js" ] && [ -f "$build_root/report/$name.js" ] || continue
		if cmp -s "$build_root/off/$name.js" "$build_root/report/$name.js"; then
			record "$name" "off=report" PASS identical
		else
			record "$name" "off=report" FAIL differs
		fi
	done
fi

echo
echo "================================================================"
printf '%-20s %-11s %-6s %s\n' "FIXTURE" "MODE" "RESULT" "DETAIL"
echo "----------------------------------------------------------------"
printf '%s' "$results" | while read -r name mode result detail; do
	printf '%-20s %-11s %-6s %s\n' "$name" "$mode" "$result" "$detail"
done
echo "----------------------------------------------------------------"
printf '%d/%d passed\n' "$((total - failures))" "$total"
echo "================================================================"

if [ "$failures" -ne 0 ]; then
	exit 1
fi
