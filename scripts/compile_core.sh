#!/usr/bin/env bash
#
# Compiles one `#![no_std]` Rust source file to JavaScript against the real
# `core`, using the sysroot scripts/build_sysroot.sh installed.
#
#   scripts/compile_core.sh <src.rs> <out.js> [--extern NAME=PATH]... [--crate-type KIND] [--crate-name NAME] [--cfg NAME]... [--search DIR]...
#
# `--extern` puts another crate on the search path, which is what a client fixture
# needs: it is compiled against `view-abi`, and `view-abi` is built by this same
# backend into an rlib first. `--crate-type` overrides the default `cdylib`, so
# that a dependency can be built as an `rlib` through the same command.
# `--crate-name` overrides the name rustc derives from the file, which every
# `src/lib.rs` derives as `lib`: two such crates in one program are otherwise
# indistinguishable to rustc and refused.
#
# `--search` adds a library search directory. `--extern` names the crates THIS
# source mentions; a crate those crates in turn depend on is found by filename on
# the search path instead, so a dependency two deep needs one of these. A proc
# macro is found the same way and must sit in a directory of its own: rustc loads
# it for the HOST, and a directory holding host rlibs beside it offers those for
# the target too and reports the wrong triple.
#
# The difference from scripts/compile.sh is the whole point of the milestone:
#
#   * `--sysroot build/sysroot` puts our own `core` on the search path, so the
#     source is an ordinary `no_std` crate rather than a `no_core` one;
#   * `--crate-type cdylib` means rustc runs `CodegenBackend::link`, which is
#     where whole-program dead code elimination and zombie reporting happen.
#     The `--emit=obj` path scripts/compile.sh uses can only ever see one crate.
#   * `--remap-path-prefix` makes the `#[track_caller]` locations `core` builds —
#     which a `#[panic_handler]` can print — repository relative, so a test's
#     expected output does not depend on where the checkout lives.
#   * `JSC_REMAP_ROOT`, when set, gives an additional source tree stable relative
#     paths. Package builders use it for sources outside this compiler checkout.
#
# Everything reachable from a `#[no_mangle]` item in this crate is emitted;
# everything else — which is nearly all of `core` — is dropped.
#
# $JS_EXTRA_ARGS is passed on to the backend as `-Cllvm-args`, exactly as
# scripts/compile.sh does. An option that changes what the backend *emits* has
# to be passed to scripts/build_sysroot.sh as well, or this crate's items and
# `core`'s disagree; see CONTRACT.md. Options that only change this crate's own
# output (`js-comments`, `js-line-comments`, `js-source-map`) are safe to pass
# here alone — they are per item, and an item is emitted by whichever crate
# codegenned it.
#
# $JSC_RUST_MIR_INLINE=on adds `-Zinline-mir` to this crate. The sysroot is
# always built with it, so this only adds the app-crate half: a `core` body
# small enough for rustc's cost model lands inside the caller instead of behind
# a call, which is what lets the backend's own passes see one body. It does not
# take part in the sysroot cache key, because it changes nothing the sysroot
# build does. It is off by default; turning it on changes the emitted JavaScript
# in every optimizer mode, so it is a build setting rather than an optimizer
# pass.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"

if [ "$#" -lt 2 ]; then
	echo "usage: compile_core.sh <src.rs> <out.js> [--extern NAME=PATH]... [--crate-type KIND] [--crate-name NAME] [--cfg NAME]... [--search DIR]..." >&2
	exit 2
fi

src=$1
out=$2
shift 2

crate_type=cdylib
crate_name=""
externs=()
cfgs=()
searches=()
while [ "$#" -gt 0 ]; do
	case "$1" in
	--extern)
		if [ "$#" -lt 2 ]; then
			echo "compile_core.sh: --extern needs a NAME=PATH" >&2
			exit 2
		fi
		externs+=(--extern "$2")
		shift 2
		;;
	--crate-type)
		if [ "$#" -lt 2 ]; then
			echo "compile_core.sh: --crate-type needs a kind" >&2
			exit 2
		fi
		crate_type=$2
		shift 2
		;;
	--search)
		if [ "$#" -lt 2 ]; then
			echo "compile_core.sh: --search needs a directory" >&2
			exit 2
		fi
		searches+=(-L "dependency=$2")
		shift 2
		;;
	--cfg)
		if [ "$#" -lt 2 ]; then
			echo "compile_core.sh: --cfg needs a name" >&2
			exit 2
		fi
		cfgs+=(--cfg "$2")
		shift 2
		;;
	--crate-name)
		if [ "$#" -lt 2 ]; then
			echo "compile_core.sh: --crate-name needs a name" >&2
			exit 2
		fi
		crate_name=$2
		shift 2
		;;
	*)
		echo "compile_core.sh: unknown option: $1" >&2
		exit 2
		;;
	esac
done

if [ ! -f "$src" ]; then
	echo "compile_core.sh: no such source file: $src" >&2
	exit 1
fi

src=$(cd -- "$(dirname -- "$src")" && pwd)/$(basename -- "$src")

out_dir=$(dirname -- "$out")
mkdir -p "$out_dir"
out_dir=$(cd -- "$out_dir" && pwd)
out="$out_dir/$(basename -- "$out")"

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac

if [ ! -f "$dylib" ]; then
	echo "compile_core.sh: backend not built: $dylib" >&2
	echo "compile_core.sh: run scripts/build.sh first" >&2
	exit 1
fi

# The consumer half of the compiler. The dylib finds it as its own sibling, which
# is where scripts/build.sh puts it; exporting the path says so outright, so a
# stale or relocated build tree fails here with a name rather than inside rustc.
: "${JSC_LINK_BIN:=$root/target/release/jsc-link}"
export JSC_LINK_BIN

if [ ! -f "$JSC_LINK_BIN" ]; then
	echo "compile_core.sh: linker not built: $JSC_LINK_BIN" >&2
	echo "compile_core.sh: run scripts/build.sh first" >&2
	exit 1
fi

case "${JSC_RUST_OPT_LEVEL:-}" in
"" | 0 | 1 | 2 | 3 | s | z) ;;
*)
	echo "compile_core.sh: invalid JSC_RUST_OPT_LEVEL: $JSC_RUST_OPT_LEVEL" >&2
	exit 2
	;;
esac

case "${JSC_RUST_MIR_INLINE:-off}" in
off | on) ;;
*)
	echo "compile_core.sh: invalid JSC_RUST_MIR_INLINE: $JSC_RUST_MIR_INLINE" >&2
	exit 2
	;;
esac

sysroot=$(jsc_resolve_sysroot \
	"$root" wasm32-unknown-unknown "$dylib")
if [ ! -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
	echo "compile_core.sh: no sysroot at $sysroot" >&2
	echo "compile_core.sh: build matching settings with scripts/build_sysroot.sh" >&2
	exit 1
fi

channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"

rm -f "$out"

stderr_file="$out_dir/.compile-core-stderr.$$"
trap 'rm -f "$stderr_file"' EXIT

# Rustc tracks the path but not the backend file's contents.
backend_args=(-Cllvm-args="js-producer=$(jsc_sha256_file "$dylib")")
if [ -n "${JS_EXTRA_ARGS:-}" ]; then
	backend_args+=(-Cllvm-args="$JS_EXTRA_ARGS")
fi

rust_opt_args=()
if [ -n "${JSC_RUST_OPT_LEVEL:-}" ]; then
	rust_opt_args=(-Copt-level="$JSC_RUST_OPT_LEVEL")
fi

mir_inline_args=()
if [ "${JSC_RUST_MIR_INLINE:-off}" = on ]; then
	mir_inline_args=(-Zinline-mir)
fi

name_args=()
if [ -n "$crate_name" ]; then
	name_args=(--crate-name "$crate_name")
fi

remap_args=(--remap-path-prefix "$root/=")
if [ -n "${JSC_REMAP_ROOT:-}" ]; then
	if [ ! -d "$JSC_REMAP_ROOT" ]; then
		echo "compile_core.sh: no such JSC_REMAP_ROOT: $JSC_REMAP_ROOT" >&2
		exit 2
	fi
	remap_root=$(cd -- "$JSC_REMAP_ROOT" && pwd)
	remap_args+=(--remap-path-prefix "$remap_root/=")
fi

set +e
(
	cd "$out_dir" &&
		rustc "+$channel" \
			-Zcodegen-backend="$dylib" \
			--edition 2021 \
			--target wasm32-unknown-unknown \
			--sysroot "$sysroot" \
			--crate-type "$crate_type" \
			${name_args[@]+"${name_args[@]}"} \
			-Ccodegen-units=1 -Cpanic=abort -Coverflow-checks=off -Cdebug-assertions=off -Cdebuginfo=0 \
			${rust_opt_args[@]+"${rust_opt_args[@]}"} \
			${mir_inline_args[@]+"${mir_inline_args[@]}"} \
			${remap_args[@]+"${remap_args[@]}"} \
			${externs[@]+"${externs[@]}"} \
			${cfgs[@]+"${cfgs[@]}"} \
			${searches[@]+"${searches[@]}"} \
			${backend_args[@]+"${backend_args[@]}"} \
			-o "$out" "$src"
) >/dev/null 2>"$stderr_file"
status=$?
set -e

if [ "$status" -ne 0 ]; then
	# rustc may have copied a provisional object onto -o before the custom link failed.
	# Never leave it looking like a successfully linked JavaScript artifact.
	rm -f -- "$out"
	echo "compile_core.sh: rustc failed (exit $status) compiling $src" >&2
	cat "$stderr_file" >&2
	exit "$status"
fi

if [ -s "$stderr_file" ]; then
	cat "$stderr_file" >&2
fi

if [ ! -f "$out" ]; then
	echo "compile_core.sh: rustc produced no output for $src" >&2
	exit 1
fi

echo "$out"
