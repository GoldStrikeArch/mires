#!/usr/bin/env bash
#
# The DOM suite: `view!` fixtures compiled to ES modules and run against the contract's stub.
#
#   scripts/dom-test.sh                  # check every fixture
#   scripts/dom-test.sh 01_simple_elements
#   UPDATE_EXPECT=1 scripts/dom-test.sh  # rewrite the goldens from the output
#   JSC_RUST_MIR_INLINE=on scripts/dom-test.sh # check traces with MIR inlining
#
# Each fixture in examples/dom-tests/ is a `#![no_std]` crate that writes its markup with
# `dom_view_client_only!` and exports `rust_entry`. It carries two expectations:
#
#   NN.js.expected     the emitted JavaScript, byte for byte
#   NN.trace.expected  the runtime calls the module makes, as JSON Lines
#
# and, where a corpus family means the same thing, one more comparison:
#
#   NN.family          the corpus family this fixture mirrors, e.g. `06-insert-children`
#
# which is compared against contract/fixtures/corpus/<family>/expected.reference.trace with
# contract/harness/compare-trace.mjs. That is the corpus's L2 -- trace parity -- and it is reported
# per fixture rather than being allowed to fail the suite, because a difference there is a finding
# about two emitters and not necessarily a regression in this one. examples/dom-tests/deltas.json
# holds the accepted ones.
#
# HOW A CLIENT FIXTURE IS COMPILED
# --------------------------------
# `dom_view!` is a proc macro, so two things have to line up that the `core` suite never needs:
#
#   1. `view-abi` is compiled by THIS backend into an rlib, since the fixture links against it;
#   2. `view-dom-macro` is built by plain cargo for the HOST, because a proc macro runs in the
#      compiler's own process and rustc loads the host dylib whatever `--target` says.
#
# The fixture is then compiled in one pass, macro and all. There is no pre-expansion step: the
# payload `static` carries no attribute for the wasm target to refuse, so nothing has to be
# rewritten between expanding and compiling. See CONTRACT.md, "Templates".
#
# THE JIR LANE
# ------------
# `js-lane=jir` in $JS_EXTRA_ARGS is passed through to every compilation here — which lane a
# program is printed from is a link step decision, so the fixtures link against the sysroot that
# is already there. Under it a fixture compares its JavaScript against
#
#   NN.jir.js.expected   the emitted JavaScript on the JIR lane
#
# and falls back to NN.js.expected when there is none, so a lane whose items all fall back (every
# one of them today) passes against the baseline goldens unchanged. The trace golden is shared:
# a lane that changed what the program DOES would be a miscompilation, not a new expectation.
# `UPDATE_EXPECT=1` under the lane writes the lane golden and never the baseline one.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

fixture_dir="$root/examples/dom-tests"
build_dir="$root/build/domtest"
log_dir="$root/build/logs"
corpus="$root/contract/fixtures/corpus"
harness="$root/contract/harness"
deltas="$fixture_dir/deltas.json"

# The module specifier the compiled program imports the DOM runtime from. It resolves inside
# $build_dir to the re-export of the contract's stub written below.
DOM_MODULE="./topcoat-dom.js"
DOM_ARGS="js-names=readable js-comments=on js-modules=esm js-dom-module=$DOM_MODULE"
DOM_OPT_MODE=${DOM_OPT_MODE:-off}
case "$DOM_OPT_MODE" in
off) ;;
report | on) DOM_ARGS="$DOM_ARGS js-opt=$DOM_OPT_MODE" ;;
*)
	echo "dom-test.sh: DOM_OPT_MODE must be off, report, or on" >&2
	exit 2
	;;
esac
if [ "${UPDATE_EXPECT:-0}" = 1 ] && { [ "$DOM_OPT_MODE" != off ] || [ "${JSC_RUST_MIR_INLINE:-off}" = on ]; }; then
	echo "dom-test.sh: expectations require optimizer off and MIR inlining off" >&2
	exit 2
fi

# The lane the caller asked for. Only `js-lane=jir` is honoured: `shadow` writes two programs and
# is what scripts/lane-shadow-test.sh runs, not a golden mode.
lane=""
case " ${JS_EXTRA_ARGS:-} " in
*" js-lane=jir "*)
	lane=jir
	DOM_ARGS="$DOM_ARGS js-lane=jir"
	echo "note: fixtures are compiled with js-lane=jir; NN.jir.js.expected wins where it exists"
	;;
esac

mkdir -p "$build_dir" "$log_dir"

# ---------------------------------------------------------------- prerequisites

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "dom-test.sh: backend build failed" >&2
		exit 1
	fi
fi

echo "==> building view-dom-macro for the host"
if ! (cd "$root" && cargo build -p view-dom-macro) >"$log_dir/dom-macro.log" 2>&1; then
	echo "dom-test.sh: could not build the proc macro" >&2
	cat "$log_dir/dom-macro.log" >&2
	exit 1
fi
case "$(uname -s)" in
Darwin) proc_macro="$root/target/debug/libview_dom_macro.dylib" ;;
*) proc_macro="$root/target/debug/libview_dom_macro.so" ;;
esac

echo "==> building view-abi through the backend"
abi_rlib="$build_dir/libview_abi.rlib"
if ! JS_EXTRA_ARGS="$DOM_ARGS" "$script_dir/compile_core.sh" \
	"$root/view-abi/src/lib.rs" "$abi_rlib" \
	--crate-type rlib >"$log_dir/dom-abi.log" 2>&1; then
	echo "dom-test.sh: could not build view-abi as an rlib" >&2
	cat "$log_dir/dom-abi.log" >&2
	exit 1
fi

# The run directory: a module package, the ES module shim, and the DOM runtime the compiled
# programs import.
printf '{ "type": "module" }\n' >"$build_dir/package.json"
if ! node "$script_dir/make-esm-shim.mjs" "$build_dir/shim.js"; then
	echo "dom-test.sh: could not derive the ES module shim" >&2
	exit 1
fi
cat >"$build_dir/topcoat-dom.js" <<'EOF'
// The DOM runtime a compiled client program imports, for the suite's purposes.
//
// Everything is the contract's recording stub, re-exported unchanged, so a call the program makes
// lands in the same trace scripts/dom-trace.mjs reads. Written by scripts/dom-test.sh.
export * from "../../contract/harness/trace.mjs";

// `createSignal` is not part of the client ABI the stub implements -- it is solid-js's, and a
// reference-compiled module reaches it as a free binding rather than as an import. It is
// implemented here rather than recorded, which is also what the reference traces show: their
// `createSignal` is a healed free binding and makes no records.
export function createSignal(initial) {
  let value = initial;
  return [
    () => value,
    (next) => {
      value = typeof next === "function" ? next(value) : next;
      return value;
    }
  ];
}
EOF

# ------------------------------------------------------------------- selection

selected=${1:-}
if [ -n "$selected" ]; then
	name=$(basename -- "$selected")
	name=${name%.rs}
	if [ ! -f "$fixture_dir/$name.rs" ]; then
		echo "dom-test.sh: no such fixture: $name" >&2
		ls "$fixture_dir"/*.rs 2>/dev/null | while read -r f; do
			echo "  $(basename -- "${f%.rs}")" >&2
		done
		exit 2
	fi
	sources=$fixture_dir/$name.rs
else
	sources=$(ls "$fixture_dir"/*.rs 2>/dev/null | sort)
	if [ -z "$sources" ]; then
		echo "dom-test.sh: no fixtures found in $fixture_dir" >&2
		exit 1
	fi
fi

results=""
failures=0
updated=0
total=0
verdicts=""

for src in $sources; do
	name=$(basename -- "${src%.rs}")
	out="$build_dir/$name.js"
	expected_js="$fixture_dir/$name.js.expected"
	# The lane's own golden, where the fixture has one. Under UPDATE_EXPECT the lane golden is
	# the one written, so a lane run can never rebaseline the baseline.
	if [ -n "$lane" ] && { [ "${UPDATE_EXPECT:-0}" = "1" ] || [ -f "$fixture_dir/$name.$lane.js.expected" ]; }; then
		expected_js="$fixture_dir/$name.$lane.js.expected"
	fi
	expected_trace="$fixture_dir/$name.trace.expected"
	actual_trace="$build_dir/$name.trace"
	total=$((total + 1))

	echo "==> $name"

	# 1. Compile the fixture, macro and all, through the backend.
	if ! JS_EXTRA_ARGS="$DOM_ARGS" "$script_dir/compile_core.sh" "$src" "$out" \
		--extern view_abi="$abi_rlib" --extern view_dom_macro="$proc_macro" 		>"$log_dir/dom-$name.compile.log" 2>&1; then
		echo "    compile failed:"
		sed 's/^/    /' "$log_dir/dom-$name.compile.log"
		results="$results$name FAIL compile-error"$'\n'
		failures=$((failures + 1))
		continue
	fi

	# 2. Run it against the recording stub.
	if ! node "$script_dir/dom-trace.mjs" "$out" "domtest/$name" >"$actual_trace" \
		2>"$log_dir/dom-$name.run.log"; then
		echo "    runtime error:"
		sed 's/^/    /' "$log_dir/dom-$name.run.log"
		head -n 1 "$actual_trace" | sed 's/^/    /'
		results="$results$name FAIL runtime-error"$'\n'
		failures=$((failures + 1))
		continue
	fi

	# A trace records where a handler was installed but does not invoke it. This fixture also
	# checks which loop iteration each installed handler captured.
	if [ "$name" = 23_for_row_handlers ]; then
		semantic_log="$log_dir/dom-$name.handlers.log"
		if ! node "$script_dir/for-handler-check.mjs" >"$semantic_log" 2>&1; then
			echo "    handler semantics changed:"
			sed 's/^/    /' "$semantic_log"
			results="$results$name FAIL handler-semantics"$'\n'
			failures=$((failures + 1))
			continue
		fi
		semantic_checks=$(grep -c '^ok  ' "$semantic_log" 2>/dev/null)
		echo "    handler semantics ok ($semantic_checks checks)"
	fi

	if [ "${UPDATE_EXPECT:-0}" = "1" ]; then
		changed=""
		if ! cmp -s "$out" "$expected_js" 2>/dev/null; then
			cp "$out" "$expected_js"
			changed="js"
		fi
		if ! cmp -s "$actual_trace" "$expected_trace" 2>/dev/null; then
			cp "$actual_trace" "$expected_trace"
			changed="${changed:+$changed+}trace"
		fi
		if [ -n "$changed" ]; then
			echo "    updated ($changed)"
			results="$results$name UPDATED $changed"$'\n'
			updated=$((updated + 1))
		else
			echo "    ok (unchanged)"
			results="$results$name PASS unchanged"$'\n'
		fi
	elif [ ! -f "$expected_js" ] || [ ! -f "$expected_trace" ]; then
		echo "    missing expectation; run UPDATE_EXPECT=1 scripts/dom-test.sh $name"
		results="$results$name FAIL no-expectation"$'\n'
		failures=$((failures + 1))
		continue
	# JavaScript goldens describe optimizer-off, MIR-inline-off emission. Other
	# configurations share the exact runtime trace and handler checks.
	elif [ "$DOM_OPT_MODE" != on ] && [ "${JSC_RUST_MIR_INLINE:-off}" != on ] && \
		! diff -u "$expected_js" "$out" >"$log_dir/dom-$name.js.diff"; then
		echo "    emitted JavaScript changed (- expected, + actual):"
		sed 's/^/    /' "$log_dir/dom-$name.js.diff"
		echo "    if this is an improvement: UPDATE_EXPECT=1 scripts/dom-test.sh $name"
		results="$results$name FAIL js-mismatch"$'\n'
		failures=$((failures + 1))
		continue
	elif ! diff -u "$expected_trace" "$actual_trace" >"$log_dir/dom-$name.trace.diff"; then
		echo "    trace changed (- expected, + actual):"
		sed 's/^/    /' "$log_dir/dom-$name.trace.diff"
		results="$results$name FAIL trace-mismatch"$'\n'
		failures=$((failures + 1))
		continue
	else
		records=$(($(wc -l <"$actual_trace") - 1))
		echo "    ok ($records runtime calls)"
		results="$results$name PASS $records calls"$'\n'
	fi

	# 3. L2: the same trace against the corpus family's reference, with the accepted deltas.
	family_file="$fixture_dir/$name.family"
	if [ -f "$family_file" ]; then
		family=$(tr -d '[:space:]' <"$family_file")
		reference="$corpus/$family/expected.reference.trace"
		if [ ! -f "$reference" ]; then
			echo "    L2: no reference trace for family $family"
			verdicts="$verdicts$name $family NO-REFERENCE"$'\n'
		else
			l2_log="$log_dir/dom-$name.l2.log"
			if node "$harness/compare-trace.mjs" "$reference" "$actual_trace" \
				--delta "$deltas" --fixture "corpus/$family" >"$l2_log" 2>&1; then
				echo "    L2 vs $family: MATCH"
				verdicts="$verdicts$name $family MATCH"$'\n'
			else
				diffs=$(grep -c '^  ' "$l2_log" 2>/dev/null)
				diffs=${diffs:-0}
				echo "    L2 vs $family: DIFFERS ($diffs lines; $l2_log)"
				verdicts="$verdicts$name $family DIFFERS($diffs-lines)"$'\n'
			fi
		fi
	fi
done

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
if [ -n "$verdicts" ]; then
	echo "----------------------------------------------------------------"
	printf '%-24s %-22s %s\n' "FIXTURE" "CORPUS FAMILY" "L2 TRACE PARITY"
	printf '%s' "$verdicts" | while read -r name family verdict; do
		printf '%-24s %-22s %s\n' "$name" "$family" "$verdict"
	done
fi
echo "================================================================"

if [ "$failures" -ne 0 ]; then
	exit 1
fi
