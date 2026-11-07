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

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

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

sysroot="$root/build/sysroot"
if [ ! -d "$sysroot/lib/rustlib/wasm32-unknown-unknown/lib" ]; then
	echo "compile_core.sh: no sysroot at $sysroot" >&2
	echo "compile_core.sh: run scripts/build_sysroot.sh first" >&2
	exit 1
fi

channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
	"$root/rust-toolchain.toml" | head -n 1)
: "${channel:=nightly}"

rm -f "$out"

stderr_file="$out_dir/.compile-core-stderr.$$"
trap 'rm -f "$stderr_file"' EXIT

# An empty array expands to nothing at all, which is what keeps the default
# command line unchanged. (The `${a[@]+...}` guard is for bash 3.2, where an
# empty array under `set -u` is an unbound variable.)
backend_args=()
if [ -n "${JS_EXTRA_ARGS:-}" ]; then
	backend_args=(-Cllvm-args="$JS_EXTRA_ARGS")
fi

name_args=()
if [ -n "$crate_name" ]; then
	name_args=(--crate-name "$crate_name")
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
			--remap-path-prefix "$root/=" \
			${externs[@]+"${externs[@]}"} \
			${cfgs[@]+"${cfgs[@]}"} \
			${searches[@]+"${searches[@]}"} \
			${backend_args[@]+"${backend_args[@]}"} \
			-o "$out" "$src"
) >/dev/null 2>"$stderr_file"
status=$?
set -e

if [ "$status" -ne 0 ]; then
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
