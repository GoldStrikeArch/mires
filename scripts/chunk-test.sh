#!/usr/bin/env bash
#
# The chunk suite: real-`core` programs split into ES module chunks, run as chunks.
#
#   scripts/chunk-test.sh                          # every examples/core-tests/NN.chunks
#   scripts/chunk-test.sh 262_chunk_owned_statics  # one
#   SKIP_BUILD=1 scripts/chunk-test.sh             # reuse the built backend
#
# A core test that carries an `NN.chunks` file is also built split. Each non-empty line that is not
# a `#` comment is one build: its `chunk:entry` pairs are handed to the compiler as
# `-Cllvm-args=js-chunk=<chunk>:<entry>`, in the order written. Every build of a fixture is run the
# same way, by scripts/run-chunks.sh, importing each chunk's entry from its own file and calling
# the entries in the order of the FIRST line. That order must be the calls the fixture's
# `rust_entry` makes, which is what lets the one `NN.expected` -- the native expectation
# scripts/test.sh holds the unsplit program to -- be the expectation of every split as well. The
# other lines ask for the same chunks in another order, which moves every chunk's index and so
# every ownership decision's iteration order without changing what the program does.
#
# What a pass proves is that the chunk files load (no name both imported and declared, no binding
# read before its module initialized it), that the imports resolve to the names the other chunks
# export, and that the split program prints what the native one does.
#
# The options are $JS_EXTRA_ARGS plus `js-modules=esm` and the chunk pairs; neither of those is an
# emit-affecting option, so the split builds link against the sysroot the unsplit suite uses, and
# JSC_RUST_OPT_LEVEL / JSC_RUST_MIR_INLINE select it exactly as they do for scripts/test.sh.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"

core_dir="$root/examples/core-tests"
build_dir="$root/build/chunks"
log_dir="$root/build/logs"
mkdir -p "$build_dir" "$log_dir"

if [ -z "${SKIP_BUILD:-}" ]; then
	echo "==> building the backend"
	if ! "$script_dir/build.sh" >"$log_dir/chunk-build.log" 2>&1; then
		echo "    failed, see $log_dir/chunk-build.log"
		exit 1
	fi
fi

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac
sysroot=$(jsc_resolve_sysroot "$root" wasm32-unknown-unknown "$dylib") || exit 1
if [ ! -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
	echo "==> building matching keyed sysroot: $sysroot"
	if ! JS_EXTRA_ARGS="$(jsc_sysroot_args)" SKIP_BUILD=1 "$script_dir/build_sysroot.sh" \
		>"$log_dir/chunk-sysroot.log" 2>&1; then
		echo "    sysroot build failed, see $log_dir/chunk-sysroot.log"
		exit 1
	fi
fi

if [ "$#" -gt 0 ]; then
	fixtures=""
	for name in "$@"; do
		name=$(basename -- "${name%.rs}")
		name=${name%.chunks}
		if [ ! -f "$core_dir/$name.chunks" ]; then
			echo "chunk-test.sh: no such chunked fixture: $name" >&2
			exit 2
		fi
		fixtures="$fixtures $name"
	done
else
	fixtures=$(ls "$core_dir"/*.chunks 2>/dev/null | sed 's#.*/##; s#\.chunks$##' | sort)
fi

pass=0
fail=0
failed=""
for name in $fixtures; do
	src="$core_dir/$name.rs"
	expected="$core_dir/$name.expected"
	calls=""
	build=0
	while IFS= read -r line || [ -n "$line" ]; do
		line=${line%%#*}
		# shellcheck disable=SC2086
		set -- $line
		[ "$#" -eq 0 ] && continue
		build=$((build + 1))
		[ -z "$calls" ] && calls="$*"
		label="$name[$build: $*]"
		out="$build_dir/$name/$build"
		rm -rf "$out"
		mkdir -p "$out"
		chunk_args=""
		for pair in "$@"; do
			chunk_args="$chunk_args js-chunk=$pair"
		done
		if ! JS_EXTRA_ARGS="${JS_EXTRA_ARGS:-} js-modules=esm$chunk_args" \
			"$script_dir/compile_core.sh" "$src" "$out/program.js" >"$out/compile.log" 2>&1; then
			echo "FAIL  $label: does not compile, see $out/compile.log"
			sed -n '1,6s/^/    /p' "$out/compile.log"
			fail=$((fail + 1))
			failed="$failed $name"
			continue
		fi
		# shellcheck disable=SC2086
		"$script_dir/run-chunks.sh" "$out/program.js" "chunks-$name-$build" $calls \
			>"$out/stdout" 2>"$out/stderr"
		status=$?
		if [ "$status" -eq 0 ] && cmp -s "$expected" "$out/stdout"; then
			echo "PASS  $label"
			pass=$((pass + 1))
		else
			echo "FAIL  $label: exit $status, see $out"
			diff "$expected" "$out/stdout" | sed -n '1,6s/^/    /p'
			sed -n '1,4s/^/    /p' "$out/stderr"
			fail=$((fail + 1))
			failed="$failed $name"
		fi
	done <"$core_dir/$name.chunks"
	if [ "$build" -eq 0 ]; then
		echo "FAIL  $name: $name.chunks asks for no build"
		fail=$((fail + 1))
		failed="$failed $name"
	fi
done

echo
echo "chunk suite: $pass passed, $fail failed"
if [ "$fail" -gt 0 ]; then
	echo "failed:$failed"
	exit 1
fi
