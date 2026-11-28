#!/usr/bin/env bash
#
# Compiles one no_core Rust source file to JavaScript with the rustc_codegen_js
# backend.
#
#   scripts/compile.sh <src.rs> <out.js>
#
# The "object file" rustc writes through the backend IS the JavaScript text.
# rustc does not always honour `-o` for `--emit=obj` (it may name the artifact
# after the crate and/or codegen unit), so the emitted artifact is normalized to
# <out.js> afterwards.
#
# The target is wasm32-unknown-unknown. Nothing is ever handed to LLVM, so the
# only thing the target decides is the data layout the backend reads off `tcx`:
# a 32-bit pointer width, which makes `usize`/`isize` exactly `u32`/`i32` and so
# representable as JS numbers (see backend/src/value.rs). Layouts, niches,
# `size_of` and the constants rustc folds all then agree with the JS side by
# construction. No wasm toolchain component is required: `--emit=obj` with a
# `#![no_core]` crate never links and never loads a sysroot rlib.
#
# `--remap-path-prefix` makes the `#[track_caller]` locations the backend emits —
# which a `#[panic_handler]` can print — repository relative, so a test's
# expected output does not depend on where the checkout lives. It is spelled the
# same way in scripts/compile_core.sh, which is what keeps a location recorded in
# one crate and the same location recorded in another spelled identically, and so
# shareable (see `js-hoist-consts` in rcgjs-object/src/opts.rs).
#
# $JS_EXTRA_ARGS is passed on to the backend as `-Cllvm-args`, the one channel a
# hot-plugged codegen backend gets (see rcgjs-object/src/opts.rs). It is empty by
# default, so the compiler is invoked exactly as it was without it. Several
# options separate with spaces:
#
#   JS_EXTRA_ARGS='js-structure=trampoline' scripts/test.sh
#
# $JSC_RUST_MIR_INLINE=on adds `-Zinline-mir`, which runs rustc's MIR inliner on
# this crate. The sysroot is always built with it (scripts/build_sysroot.sh), so
# this only ever adds the app-crate half; it changes what rustc hands the backend
# and so changes the emitted JavaScript in every optimizer mode. It is off by
# default, which is what keeps the goldens and the off/report byte identity
# untouched.
#
# With `js-source-map=on` the backend also writes <out.js>.map beside the output
# and ends the JavaScript with a sourceMappingURL comment naming it. The map is
# written to the `-o` path, so it is only correct on the normal path below; if
# rustc names the artifact itself and the fallback rename at the bottom of this
# script has to run, the map keeps the name rustc chose.
#
# $JSC_PANIC=unwind selects the unwind build mode: targets/mires32-unwind.json
# (wasm32-unknown-unknown with an `unwind` panic strategy), `-Cpanic=unwind`,
# and `js-cleanup=off` after $JS_EXTRA_ARGS (see `jsc_panic` in
# scripts/sysroot-cache.sh). Unset, the command line is exactly the default one.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"

if [ "$#" -ne 2 ]; then
	echo "usage: compile.sh <src.rs> <out.js>" >&2
	exit 2
fi

src=$1
out=$2

if [ ! -f "$src" ]; then
	echo "compile.sh: no such source file: $src" >&2
	exit 1
fi

# Absolute paths: rustc runs with its cwd set to the output directory so any
# stray artifacts land in build/ rather than in the repo.
src=$(cd -- "$(dirname -- "$src")" && pwd)/$(basename -- "$src")

out_dir=$(dirname -- "$out")
mkdir -p "$out_dir"
out_dir=$(cd -- "$out_dir" && pwd)
out_name=$(basename -- "$out")
out="$out_dir/$out_name"

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac

if [ ! -f "$dylib" ]; then
	echo "compile.sh: backend not built: $dylib" >&2
	echo "compile.sh: run scripts/build.sh first" >&2
	exit 1
fi

# The consumer half of the compiler. The dylib finds it as its own sibling, which
# is where scripts/build.sh puts it; exporting the path says so outright, so a
# stale or relocated build tree fails here with a name rather than inside rustc.
: "${JSC_LINK_BIN:=$root/target/release/jsc-link}"
export JSC_LINK_BIN

if [ ! -f "$JSC_LINK_BIN" ]; then
	echo "compile.sh: linker not built: $JSC_LINK_BIN" >&2
	echo "compile.sh: run scripts/build.sh first" >&2
	exit 1
fi

jsc_select_mode "$root" || exit 2

# rustc derives the crate name from the file stem, with `-` mapped to `_`.
crate=$(basename -- "$src" .rs | tr '-' '_')

# The pinned nightly, named explicitly so compile.sh works from any cwd (a
# rust-toolchain.toml only applies inside the project directory).
channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"

rm -f "$out"

stamp="$out_dir/.compile-stamp.$$"
: >"$stamp"
# This trap replaces the one integration-lock.sh installed, so it releases the lock itself. A nested
# invocation runs under its caller's lock, defines no release function, and leaves the lock alone.
cleanup() {
	rm -f "$stamp"
	if declare -F release_integration_lock >/dev/null; then release_integration_lock; fi
}
trap cleanup EXIT

stderr_file="$out_dir/.compile-stderr.$$"

# Rustc tracks the path but not the backend file's contents.
backend_args=(-Cllvm-args="js-producer=$(jsc_sha256_file "$dylib")")
extra_args=$(jsc_mode_backend_args "${JS_EXTRA_ARGS:-}")
if [ -n "$extra_args" ]; then
	backend_args+=(-Cllvm-args="$extra_args")
fi

rust_opt_args=()
if [ -n "${JSC_RUST_OPT_LEVEL:-}" ]; then
	case "$JSC_RUST_OPT_LEVEL" in
	0 | 1 | 2 | 3 | s | z) rust_opt_args=(-Copt-level="$JSC_RUST_OPT_LEVEL") ;;
	*)
		echo "compile.sh: invalid JSC_RUST_OPT_LEVEL: $JSC_RUST_OPT_LEVEL" >&2
		exit 2
		;;
	esac
fi

mir_inline_args=()
case "${JSC_RUST_MIR_INLINE:-off}" in
off) ;;
on) mir_inline_args=(-Zinline-mir) ;;
*)
	echo "compile.sh: invalid JSC_RUST_MIR_INLINE: $JSC_RUST_MIR_INLINE" >&2
	exit 2
	;;
esac

set +e
(
	cd "$out_dir" &&
		rustc "+$channel" \
			-Zcodegen-backend="$dylib" \
			--edition 2021 --crate-type lib --emit=obj \
			--target "$jsc_target" \
			${jsc_unstable_args[@]+"${jsc_unstable_args[@]}"} \
			-Ccodegen-units=1 "$jsc_panic_flag" -Coverflow-checks=off -Cdebug-assertions=off -Cdebuginfo=0 \
			${rust_opt_args[@]+"${rust_opt_args[@]}"} \
			${mir_inline_args[@]+"${mir_inline_args[@]}"} \
			--remap-path-prefix "$root/=" \
			--cfg mires --check-cfg='cfg(mires)' \
			${backend_args[@]+"${backend_args[@]}"} \
			-o "$out" "$src"
) >/dev/null 2>"$stderr_file"
status=$?
set -e

if [ "$status" -ne 0 ]; then
	echo "compile.sh: rustc failed (exit $status) compiling $src" >&2
	cat "$stderr_file" >&2
	rm -f "$stderr_file"
	exit "$status"
fi

# Surface warnings, but do not fail on them.
if [ -s "$stderr_file" ]; then
	cat "$stderr_file" >&2
fi
rm -f "$stderr_file"

if [ ! -f "$out" ]; then
	# rustc named the artifact itself: pick the newest object emitted for this
	# crate during this run and move it into place.
	# Only object-shaped artifacts count: rustc names them `<crate>.o`,
	# `<crate>.<hash>-cgu.N.rcgu.o` or similar. Matching `<crate>.*` outright
	# would also catch unrelated files a caller keeps in the same directory.
	candidates=$(find "$out_dir" -maxdepth 1 -type f -newer "$stamp" \
		\( -name "$crate" -o -name "$crate.o" -o -name "$crate.js" \
		-o -name "$crate.*.o" -o -name "$crate-*.o" \) \
		! -name "$out_name" ! -name '.compile-*' 2>/dev/null)

	if [ -z "$candidates" ]; then
		echo "compile.sh: rustc produced no output for $src" >&2
		echo "compile.sh: no object artifact for crate '$crate' appeared in $out_dir" >&2
		exit 1
	fi

	newest=$(printf '%s\n' "$candidates" | tr '\n' '\0' | xargs -0 ls -t | head -n 1)
	mv -f "$newest" "$out"
fi

echo "$out"
