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
# the second links the first. `js-extern-macro` is a proc macro and is built by plain cargo for
# the host. The DOM module is written below: the executor imports `getOwner` and `runWithOwner` from
# it, and `runWithOwner` is the one name a client DOM module owes beyond the client ABI's 48 plus
# `createSignal` (it is exported by `solid-js` and not re-exported by `solid-js/web`).

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

fixture_dir="$root/examples/async-tests"
build_dir="$root/build/asynctest"
log_dir="$root/build/logs"

DOM_MODULE="./mires-dom.js"
ASYNC_ARGS="js-names=readable js-comments=on js-modules=esm js-dom-module=$DOM_MODULE"

mkdir -p "$build_dir" "$log_dir"

# ---------------------------------------------------------------- prerequisites

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> building backend"
	if ! "$script_dir/build.sh" >/dev/null; then
		echo "async-test.sh: backend build failed" >&2
		exit 1
	fi
fi

echo "==> building the proc macro for the host"
if ! (cd "$root" && cargo build -p js-extern-macro) >"$log_dir/async-macro.log" 2>&1; then
	echo "async-test.sh: could not build the proc macro" >&2
	cat "$log_dir/async-macro.log" >&2
	exit 1
fi
case "$(uname -s)" in
Darwin) suffix=dylib ;;
*) suffix=so ;;
esac
extern_macro="$root/target/debug/libjs_extern_macro.$suffix"

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

echo "==> building view-async through the backend"
async_rlib="$build_dir/libview_async.rlib"
if ! JS_EXTRA_ARGS="$ASYNC_ARGS" "$script_dir/compile_core.sh" "$root/view-async/src/lib.rs" "$async_rlib" \
	--crate-type rlib --crate-name view_async --extern view_abi="$abi_rlib" \
	>"$log_dir/async-rlib.log" 2>&1; then
	echo "async-test.sh: could not build view-async as an rlib" >&2
	cat "$log_dir/async-rlib.log" >&2
	exit 1
fi

# The run directory: a module package, the ES module shim, and the DOM module the executor's owner
# markers import from.
printf '{ "type": "module" }\n' >"$build_dir/package.json"
if ! node "$script_dir/make-esm-shim.mjs" "$build_dir/shim.js"; then
	echo "async-test.sh: could not derive the ES module shim" >&2
	exit 1
fi
cat >"$build_dir/mires-dom.js" <<'EOF'
// The DOM runtime the executor's owner markers import from, for this suite's purposes.
//
// `getOwner` and `runWithOwner` are the whole surface an executor needs, and this is the smallest
// implementation of the contract they are held to: the current owner is a module-level variable,
// `runWithOwner` installs one for the duration of a synchronous call and restores whatever was
// there in a `finally`. That is what solid's own does, minus the `Listener` it restores alongside,
// which nothing here reads.
//
// The point of implementing rather than recording them is that the assertions are about the
// executor: the fixtures ask whether a continuation ran under the owner the spawn captured, and a
// stub that only recorded the calls could not answer.
//
// Written by scripts/async-test.sh.
let current = null;

export function getOwner() {
  return current;
}

export function runWithOwner(owner, run) {
  const previous = current;
  current = owner;
  try {
    return run();
  } finally {
    current = previous;
  }
}

// A signal is the `[read, write]` pair the value model keeps in a `Sig<T>` handle, so a fixture
// that writes one from a continuation hands the driver something it can read.
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

	if ! JS_EXTRA_ARGS="$ASYNC_ARGS" "$script_dir/compile_core.sh" "$src" "$out" \
		--search "$build_dir" \
		--extern view_abi="$abi_rlib" --extern view_async="$async_rlib" \
		--extern js_extern_macro="$extern_macro" \
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
