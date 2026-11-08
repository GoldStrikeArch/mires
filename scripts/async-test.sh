#!/usr/bin/env bash
#
# The executor suite: Rust futures spawned, polled and resumed inside a compiled program.
#
#   scripts/async-test.sh                  # check every fixture
#   scripts/async-test.sh 01_microtask
#
# Each fixture in examples/async-tests/ is a `#![no_std]` crate linked against `view-abi` and
# `view-async` and compiled to an ES module. scripts/async-check.mjs then installs the host
# interface examples/async-tests/prelude.rs declares, asserts the emission's shape, drives each
# entry point and asserts the sequence of steps the executor produced.
#
# WHY THERE IS NO BYTE FOR BYTE GOLDEN HERE
# -----------------------------------------
# A fixture that spawns anything links `Vec`, `Rc` and the allocator, so its emission is ~45KB of
# `core` around ~40 lines of executor. Those 45KB are pinned already, by the emit and core suites,
# and a golden of them here would rebaseline on every unrelated `core` change while hiding the
# lines that are actually about the executor. So the driver asserts the SHAPE instead -- that the
# scheduler is a microtask, that the owner is captured through the DOM module, that the settled
# bridge is the shim's -- and then asserts what running it does.
#
# HOW A FIXTURE IS COMPILED
# -------------------------
# `view-abi` and `view-async` are compiled by THIS backend into rlibs first, in that order, since
# the second links the first. `js-macro` is a proc macro and is built by plain cargo for the
# host. The suite uses the vendored DOM runtime so owner cleanup and
# re-entry execute in the same reactive graph as deployed applications.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"

fixture_dir="$root/examples/async-tests"
build_dir="$root/build/asynctest"
log_dir="$root/build/logs"

DOM_MODULE="./topcoat-dom.js"
ASYNC_ARGS="js-names=readable js-comments=on js-modules=esm js-dom-module=$DOM_MODULE"
ASYNC_OPT_MODE=${ASYNC_OPT_MODE:-off}
case "$ASYNC_OPT_MODE" in
off) ;;
report | on) ASYNC_ARGS="$ASYNC_ARGS js-opt=$ASYNC_OPT_MODE" ;;
*)
	echo "async-test.sh: ASYNC_OPT_MODE must be off, report, or on" >&2
	exit 2
	;;
esac
export ASYNC_OPT_MODE

# The lane the caller asked for. `js-lane` is a link step decision and not emit-affecting, so
# nothing here needs a sysroot of its own; only `jir` is honoured, since `shadow` writes two
# programs and scripts/async-check.mjs drives one directory of them.
case " ${JS_EXTRA_ARGS:-} " in
*" js-lane=jir "*)
	ASYNC_ARGS="$ASYNC_ARGS js-lane=jir"
	echo "note: fixtures are compiled with js-lane=jir"
	;;
esac

mkdir -p "$build_dir" "$log_dir"

# ---------------------------------------------------------------- prerequisites

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "async-test.sh: backend build failed" >&2
		exit 1
	fi
fi

echo "==> building the proc macros for the host"
if ! (cd "$root" && cargo build -p js-macro -p js-extern-macro) >"$log_dir/async-macro.log" 2>&1; then
	echo "async-test.sh: could not build the proc macros" >&2
	cat "$log_dir/async-macro.log" >&2
	exit 1
fi
case "$(uname -s)" in
Darwin) suffix=dylib ;;
*) suffix=so ;;
esac
js_macro="$root/target/debug/libjs_macro.$suffix"
extern_macro="$root/target/debug/libjs_extern_macro.$suffix"

# A directory holding nothing but proc macros. `topcoat-dom` uses `js!{}`, so rustc has to
# find that macro while loading `topcoat-dom`'s metadata, and it finds a transitive dependency by
# filename on the search path rather than through `--extern`. The directory must hold no rlibs:
# `target/debug` has HOST builds of `view-abi` in it, and rustc offers those for the target and
# then reports the wrong triple.
macro_dir="$build_dir/macros"
mkdir -p "$macro_dir"
cp "$js_macro" "$extern_macro" "$macro_dir/"

# The DOM module specifier is emit affecting and the executor's own crate emits imports of it, so
# the rlibs are built with the same arguments the fixtures are. `view-abi` has none of its own --
# every marker in it is intercepted at the call site -- but building the pair the same way is one
# less thing to be wrong.
echo "==> building view-abi through the backend"
abi_rlib="$build_dir/libview_abi.rlib"
if ! JS_EXTRA_ARGS="$ASYNC_ARGS" "$script_dir/compile_core.sh" "$root/view-abi/src/lib.rs" "$abi_rlib" \
	--crate-type rlib --crate-name view_abi >"$log_dir/async-abi.log" 2>&1; then
	echo "async-test.sh: could not build view-abi as an rlib" >&2
	cat "$log_dir/async-abi.log" >&2
	exit 1
fi

echo "==> building topcoat-runtime-macro for the host"
# The FRAMEWORK's own proc macro, out of the root workspace: fixture 06 drives the real
# `#[procedure(serde)]` expansion rather than a copy of it.
#
# Built with the SPIKE's pinned nightly and into a target directory of its own. rustc loads a
# proc-macro dylib into its own process, so one built by another compiler is refused with "extern
# location is of an unknown type"; the root workspace is on stable, which is exactly that case.
channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"
#
# `--features discover` is NOT optional here, and the reason is a defect rather than a preference:
# with the feature off, the grammar's `registration` is `None`, `quote!` interpolates nothing for
# it, and the `#[cfg(not(topcoat_client))]` written above it falls through onto the NEXT item --
# which is the client half. The client function then carries both halves of the cfg and is stripped
# whatever the flag says. See build/logs/wave5-backend-report.md; the fix is one line in
# crates/topcoat-runtime/grammar/src/procedure.rs and is the framework's to make.
if ! (cd "$root/../.." && cargo "+$channel" build -p topcoat-runtime-macro --features discover \
	--target-dir "$root/build/hostmacro-discover") >"$log_dir/async-procedure-macro.log" 2>&1; then
	echo "async-test.sh: could not build topcoat-runtime-macro" >&2
	cat "$log_dir/async-procedure-macro.log" >&2
	exit 1
fi
case "$(uname -s)" in
Darwin) procedure_macro="$root/build/hostmacro-discover/debug/libtopcoat_runtime_macro.dylib" ;;
*) procedure_macro="$root/build/hostmacro-discover/debug/libtopcoat_runtime_macro.so" ;;
esac

echo "==> building view-async through the backend"
async_rlib="$build_dir/libview_async.rlib"
if ! JS_EXTRA_ARGS="$ASYNC_ARGS" "$script_dir/compile_core.sh" "$root/view-async/src/lib.rs" "$async_rlib" \
	--crate-type rlib --crate-name view_async --extern view_abi="$abi_rlib" \
	>"$log_dir/async-rlib.log" 2>&1; then
	echo "async-test.sh: could not build view-async as an rlib" >&2
	cat "$log_dir/async-rlib.log" >&2
	exit 1
fi

echo "==> building topcoat-dom through the backend"
dom_rlib="$build_dir/libtopcoat_dom.rlib"
if ! JS_EXTRA_ARGS="$ASYNC_ARGS" "$script_dir/compile_core.sh" "$root/topcoat-dom/src/lib.rs" \
	"$dom_rlib" --crate-type rlib --crate-name topcoat_dom \
	--search "$build_dir" --search "$macro_dir" \
	--extern view_abi="$abi_rlib" --extern view_async="$async_rlib" \
	--extern js_macro="$js_macro" >"$log_dir/async-dom.log" 2>&1; then
	echo "async-test.sh: could not build topcoat-dom as an rlib" >&2
	cat "$log_dir/async-dom.log" >&2
	exit 1
fi

# The run directory: a module package, the ES module shim, and the DOM module the executor's owner
# markers import from.
printf '{ "type": "module" }\n' >"$build_dir/package.json"
if ! node "$script_dir/make-esm-shim.mjs" "$build_dir/shim.js"; then
	echo "async-test.sh: could not derive the ES module shim" >&2
	exit 1
fi
cp "$root/runtime/dom/dist/topcoat-dom.js" "$build_dir/topcoat-dom.js"

# ------------------------------------------------------------------- selection

selected=${1:-}
if [ -n "$selected" ]; then
	name=$(basename -- "$selected")
	name=${name%.rs}
	if [ ! -f "$fixture_dir/$name.rs" ]; then
		echo "async-test.sh: no such fixture: $name" >&2
		exit 2
	fi
	sources=$fixture_dir/$name.rs
else
	sources=$(ls "$fixture_dir"/[0-9]*.rs 2>/dev/null | sort)
	if [ -z "$sources" ]; then
		echo "async-test.sh: no fixtures found in $fixture_dir" >&2
		exit 1
	fi
fi

results=""
failures=0
total=0

for src in $sources; do
	name=$(basename -- "${src%.rs}")
	out="$build_dir/$name.js"
	total=$((total + 1))

	echo "==> $name"

	# Direct rustc fixtures have no Cargo package environment. Give the procedure
	# macro a stable synthetic package identity without changing facade resolution.
	if ! TOPCOAT_PROCEDURE_MANIFEST_DIR="$root" TOPCOAT_PROCEDURE_PKG_NAME="async-$name" \
		JS_EXTRA_ARGS="$ASYNC_ARGS" "$script_dir/compile_core.sh" "$src" "$out" \
		--cfg topcoat_client --search "$build_dir" --search "$macro_dir" \
		--extern view_abi="$abi_rlib" --extern view_async="$async_rlib" \
		--extern topcoat_dom="$dom_rlib" --extern js_macro="$js_macro" \
		--extern js_extern_macro="$extern_macro" \
		--extern topcoat_runtime_macro="$procedure_macro" \
		>"$log_dir/async-$name.compile.log" 2>&1; then
		echo "    compile failed:"
		sed 's/^/    /' "$log_dir/async-$name.compile.log"
		results="$results$name FAIL compile-error"$'\n'
		failures=$((failures + 1))
		continue
	fi

	echo "    ok (compiled)"
	results="$results$name PASS compiled"$'\n'
done

# ------------------------------------------------------- what the executor did

echo "==> behaviour"
if node "$script_dir/async-check.mjs" >"$log_dir/async-behaviour.log" 2>&1; then
	checks=$(grep -c '^ok  ' "$log_dir/async-behaviour.log")
	echo "    ok ($checks checks over the spawned tasks)"
	results="$results behaviour PASS $checks-checks"$'\n'
	total=$((total + 1))
else
	echo "    behaviour failed:"
	grep -E '^FAIL|failed' "$log_dir/async-behaviour.log" | sed 's/^/    /'
	results="$results behaviour FAIL see-$log_dir/async-behaviour.log"$'\n'
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
printf '%d/%d passed\n' "$((total - failures))" "$total"
echo "================================================================"

if [ "$failures" -ne 0 ]; then
	exit 1
fi
