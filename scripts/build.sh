#!/usr/bin/env bash
#
# Builds the rustc_codegen_js backend dylib and the jsc-link binary, and prints
# both paths (the dylib first, so a caller reading one line still gets it).
#
#   scripts/build.sh [extra cargo args...]
#
# The compiler is two programs now: the dylib rustc loads, which turns MIR into
# object files, and `jsc-link`, which turns object files into a program. The
# dylib execs its sibling, so the two are always built together.
#
# The nightly used is pinned by ./rust-toolchain.toml, so plain `cargo` picks it
# up automatically.
#
# Only those two packages are built. The workspace holds crates that have nothing
# to do with compiling Rust to JavaScript (the DOM runtime and its macro), and a
# script whose whole job is to produce the compiler should not fail because one of
# those does not compile.

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)

case "$(uname -s)" in
Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
esac

linker="$root/target/release/jsc-link"

cd "$root"
cargo build --release -p rustc_codegen_mires -p jsc-link "$@"

if [ ! -f "$dylib" ]; then
	echo "build.sh: cargo succeeded but $dylib is missing" >&2
	echo "build.sh: check that backend/Cargo.toml sets crate-type = [\"dylib\"]" >&2
	exit 1
fi

if [ ! -f "$linker" ]; then
	echo "build.sh: cargo succeeded but $linker is missing" >&2
	echo "build.sh: check that jsc-link/Cargo.toml declares [[bin]] jsc-link" >&2
	exit 1
fi

echo "$dylib"
echo "$linker"
