#!/usr/bin/env bash

# Shared cache-key logic for build_sysroot.sh, compile_core.sh, and mires-build.
# Backend options are whitespace-delimited everywhere else in this toolchain, so
# sorting those same words gives equivalent invocations one stable cache key.

jsc_normalize_opt_passes() {
	local value=${1:-}
	case "$value" in
	all | none)
		printf '%s' "$value"
		return
		;;
	esac
	# The registered passes in compiler execution order. A normalized list names the selected
	# ones in that order, so two spellings of one selection share a cache key. This must match
	# the backend's OptimizerPass::ORDERED and mires-build's normalize_optimizer_passes.
	local ordered="sroa ast-dce future-poller native-representations link-dedup"
	local requested=" "
	local pass
	local old_ifs=$IFS
	IFS=,
	for pass in $value; do
		case " $ordered " in
		*" $pass "*) requested="$requested$pass " ;;
		*)
			IFS=$old_ifs
			printf '%s' "$value"
			return
			;;
		esac
	done
	IFS=$old_ifs
	local selected=""
	local count=0
	local total=0
	for pass in $ordered; do
		total=$((total + 1))
		case "$requested" in
		*" $pass "*)
			selected="$selected${selected:+,}$pass"
			count=$((count + 1))
			;;
		esac
	done
	if [ "$count" -eq 0 ]; then
		printf 'none'
	elif [ "$count" -eq "$total" ]; then
		printf 'all'
	else
		printf '%s' "$selected"
	fi
}

# The emit-affecting options a `js-profile=` alias names.
#
# The alias itself is never part of the cache key: it expands here, so a build driven by
# `js-profile=release-size` and one spelling the same options out land on one sysroot instead of
# two identical ones. This list must match the backend's `Profile::apply` (rcgjs-object/src/opts.rs)
# and mires-build's `Profile::emit_args`. The link-only half of a profile — the lane it selects —
# is deliberately not expanded here: it changes nothing an object file contains.
#
# An unrecognized profile expands to itself, which keeps a typo out of the default sysroot's key.
# The compilation refuses it anyway; this only stops it from silently sharing a cache entry.
jsc_profile_emit_args() {
	case "${1:-}" in
	dev) printf 'js-opt=off' ;;
	report) printf 'js-opt=report' ;;
	release-size | release-balanced) printf 'js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=on' ;;
	release-speed) printf 'js-cleanup=on js-ecma-target=2018 js-minify=locals js-opt=on' ;;
	*) printf 'js-profile=%s' "${1:-}" ;;
	esac
}

jsc_normalize_args() {
	local args=${1:-}
	if [ -z "$args" ]; then
		return
	fi
	(
		set -f
		for arg in $args; do
			case "$arg" in
			js-opt-passes=*)
				arg="js-opt-passes=$(jsc_normalize_opt_passes "${arg#*=}")"
				;;
			esac
			printf '%s\n' "$arg"
		done
	) | awk '{ key = $0; sub(/=.*/, "", key); if ($0 == "js-opt-passes=all") delete value[key]; else value[key] = $0 }
		END { for (key in value) print value[key] }' |
		LC_ALL=C sort | awk 'NR > 1 { printf " " } { printf "%s", $0 }'
}

# The emit-affecting subset of a backend option string, normalized into a cache key.
#
# An option is emit-affecting when it can change what an object file *contains*, which is the
# whole of what the sysroot's `core` and the crate linking it have to agree on. `js-jir` is one:
# it decides whether an object carries the JIR-S body of each item it defines. The lane options
# are not — every object carries both lanes and choosing between them is a link step decision —
# so `js-lane`, `js-lane-report` and `js-jir-dump-conflicts` are absent from this list on
# purpose, and a build that only changes them reuses the sysroot it already has.
jsc_normalize_emit_args() {
	local args=${1:-}
	local emit=""
	local profile=""
	local arg
	for arg in $args; do
		case "$arg" in
		js-profile=*)
			profile="$profile${profile:+ }$(jsc_profile_emit_args "${arg#*=}")"
			;;
		js-names=* | js-structure=* | js-queue=* | js-switch=* | \
			js-scoped-lets | js-scoped-lets=* | js-minify=* | \
			js-ecma-target=* | \
			js-hoist-consts=* | js-line-comments | js-line-comments=* | js-opt=* | \
			js-opt-passes=* | js-jir=* | js-cleanup=*)
			emit="$emit${emit:+ }$arg"
			;;
		esac
	done
	# Every profile expansion goes in front of every explicit option, wherever the profile was
	# written: normalization keeps the last value given for an option, and an option spelled out
	# alongside a profile overrides it. That is the rule the backend parses by.
	jsc_normalize_args "$profile${profile:+ }$emit"
}

# The panic strategy `JSC_PANIC` selects: `unwind` (unset, the default) or `abort` (opt-in override).
#
# `unwind` compiles the sysroot and every crate for targets/mires32-unwind.json, whose panic
# strategy is `unwind`, with `-Cpanic=unwind`: rustc's optimized MIR keeps its unwind edges and the
# backend lowers those instead, so `js-cleanup=off` follows the caller's backend options
# (`jsc_mode_backend_args`). `abort` is the legacy `-Cpanic=abort` mode on wasm32-unknown-unknown,
# where host-exception cleanup is re-derived by the backend from unoptimized MIR (`js-cleanup=on`).
jsc_panic() {
	case "${JSC_PANIC:-}" in
	"" | unwind) printf 'unwind' ;;
	abort) printf 'abort' ;;
	*)
		echo "sysroot-cache.sh: invalid JSC_PANIC: $JSC_PANIC (expected unset, abort or unwind)" >&2
		return 1
		;;
	esac
}

# The target spec the unwind mode compiles for.
jsc_unwind_target_spec() {
	printf '%s/targets/mires32-unwind.json' "$1"
}

# Sets, for the scripts that run rustc or cargo, what the mode compiles with:
#
#   jsc_panic_mode     `abort` or `unwind`;
#   jsc_panic_flag     `-Cpanic=abort` or `-Cpanic=unwind`;
#   jsc_target         what `--target` takes: the triple, or the spec file's path;
#   jsc_target_name    the name rustc and cargo give the target: the triple, or the spec file's
#                      stem. It names the sysroot's `lib/rustlib/<name>/lib` and cargo's
#                      `target/<name>/`, and it is the `target` a cache key records;
#   jsc_unstable_args  rustc's `-Zunstable-options`, which a target spec file requires;
#   jsc_cargo_args     cargo's `-Zjson-target-spec`, likewise.
#
# In `abort` mode the two argument lists are empty, so a command line is exactly what it was.
jsc_select_mode() {
	local root=$1
	jsc_panic_mode=$(jsc_panic) || return 1
	case "$jsc_panic_mode" in
	abort)
		jsc_target=wasm32-unknown-unknown
		jsc_target_name=wasm32-unknown-unknown
		jsc_unstable_args=()
		jsc_cargo_args=()
		;;
	unwind)
		jsc_target=$(jsc_unwind_target_spec "$root")
		if [ ! -f "$jsc_target" ]; then
			echo "sysroot-cache.sh: JSC_PANIC=unwind needs the target spec $jsc_target" >&2
			return 1
		fi
		jsc_target_name=$(basename -- "$jsc_target" .json)
		jsc_unstable_args=(-Zunstable-options)
		jsc_cargo_args=(-Zjson-target-spec)
		;;
	esac
	jsc_panic_flag=-Cpanic=$jsc_panic_mode
}

# The caller's backend options as the mode compiles with them: unchanged in `abort` mode, and with
# `js-cleanup=off` appended in `unwind` mode. Appended, so that it overrides a `js-cleanup=on` or a
# release profile anywhere before it (the last value of an option wins, and an explicit option beats
# a profile, which is how the backend parses), and a suite that pins `js-cleanup=on` runs in either
# mode unchanged. This is the one place that override is made.
jsc_mode_backend_args() {
	local args=${1:-}
	local panic
	panic=$(jsc_panic) || return 1
	if [ "$panic" = unwind ]; then
		printf '%s' "$args${args:+ }js-cleanup=off"
	else
		printf '%s' "$args"
	fi
}

jsc_sysroot_args() {
	local args
	if [ "${JSC_SYSROOT_ARGS+x}" = x ]; then
		args=$(jsc_mode_backend_args "$JSC_SYSROOT_ARGS") || return 1
	else
		args=$(jsc_mode_backend_args "${JS_EXTRA_ARGS:-}") || return 1
	fi
	jsc_normalize_emit_args "$args"
}

jsc_sha256() {
	if command -v shasum >/dev/null 2>&1; then
		shasum -a 256 | awk '{ print $1 }'
	elif command -v sha256sum >/dev/null 2>&1; then
		sha256sum | awk '{ print $1 }'
	else
		openssl dgst -sha256 | awk '{ print $NF }'
	fi
}

jsc_sha256_file() {
	if command -v shasum >/dev/null 2>&1; then
		shasum -a 256 "$1" | awk '{ print $1 }'
	elif command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | awk '{ print $1 }'
	else
		openssl dgst -sha256 "$1" | awk '{ print $NF }'
	fi
}

# The digest of the `*.patch` files in `<root>/patches`, or in the directory given second
# (`patches/unwind`, which only the unwind mode applies).
jsc_patches_digest() {
	local root=$1
	local directory=${2:-$root/patches}
	local found=0
	local patch
	{
		for patch in "$directory"/*.patch; do
			[ -f "$patch" ] || continue
			found=1
			printf '%s %s\n' "$(basename -- "$patch")" "$(jsc_sha256_file "$patch")"
		done
		[ "$found" = 1 ] || printf 'none\n'
	} | jsc_sha256
}

# The digest of `mires-std/Cargo.toml` and every Rust source file under `mires-std/src/`.
jsc_mires_std_digest() {
	local root=$1
	local dir="$root/mires-std"
	local found=0
	local file
	{
		if [ -f "$dir/Cargo.toml" ]; then
			found=1
			printf 'Cargo.toml %s\n' "$(jsc_sha256_file "$dir/Cargo.toml")"
		fi
		if [ -d "$dir/src" ]; then
			while IFS= read -r file; do
				[ -f "$file" ] || continue
				found=1
				printf '%s %s\n' "${file#"$dir"/}" "$(jsc_sha256_file "$file")"
			done < <(find "$dir/src" -type f -name '*.rs' | LC_ALL=C sort)
		fi
		[ "$found" = 1 ] || printf 'none\n'
	} | jsc_sha256
}

# The digest of `mires-test/Cargo.toml` and every Rust source file under `mires-test/src/`.
jsc_mires_test_digest() {
	local root=$1
	local dir="$root/mires-test"
	local found=0
	local file
	{
		if [ -f "$dir/Cargo.toml" ]; then
			found=1
			printf 'Cargo.toml %s\n' "$(jsc_sha256_file "$dir/Cargo.toml")"
		fi
		if [ -d "$dir/src" ]; then
			while IFS= read -r file; do
				[ -f "$file" ] || continue
				found=1
				printf '%s %s\n' "${file#"$dir"/}" "$(jsc_sha256_file "$file")"
			done < <(find "$dir/src" -type f -name '*.rs' | LC_ALL=C sort)
		fi
		[ "$found" = 1 ] || printf 'none\n'
	} | jsc_sha256
}

jsc_pinned_channel() {
	local root=$1
	local channel
	channel=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\(.*\)".*/\1/p' \
		"$root/rust-toolchain.toml" | head -n 1)
	printf '%s' "${channel:-nightly}"
}

jsc_sysroot_key() {
	local root=$1
	local target=${2:-wasm32-unknown-unknown}
	local dylib=$3
	local args
	local channel
	local rustc_identity
	local rust_opt=${JSC_RUST_OPT_LEVEL:-default}
	local panic
	local spec

	if [ ! -f "$dylib" ]; then
		echo "sysroot-cache.sh: backend not built: $dylib" >&2
		return 1
	fi
	panic=$(jsc_panic) || return 1
	spec=$(jsc_unwind_target_spec "$root")
	if [ "$panic" = unwind ] && [ ! -f "$spec" ]; then
		echo "sysroot-cache.sh: JSC_PANIC=unwind needs the target spec $spec" >&2
		return 1
	fi
	channel=$(jsc_pinned_channel "$root")
	args=$(jsc_sysroot_args) || return 1
	if ! rustc_identity=$(rustc "+$channel" -vV 2>/dev/null); then
		echo "sysroot-cache.sh: cannot identify pinned toolchain $channel" >&2
		return 1
	fi

	{
		printf 'cache-format=1\n'
		printf 'backend=%s\n' "$(jsc_sha256_file "$dylib")"
		printf 'patches=%s\n' "$(jsc_patches_digest "$root")"
		printf 'mires-std=%s\n' "$(jsc_mires_std_digest "$root")"
		printf 'mires-test=%s\n' "$(jsc_mires_test_digest "$root")"
		printf 'channel=%s\n' "$channel"
		printf 'rustc=%s\n' "$rustc_identity"
		printf 'target=%s\n' "$target"
		printf 'rust-opt=%s\n' "$rust_opt"
		printf 'js-args=%s\n' "$args"
		# The unwind mode's own inputs. They are absent from an `abort` key, so every key computed
		# before the mode existed is still the key computed now.
		if [ "$panic" = unwind ]; then
			printf 'panic=unwind\n'
			printf 'target-spec=%s\n' "$(jsc_sha256_file "$spec")"
			printf 'unwind-patches=%s\n' "$(jsc_patches_digest "$root" "$root/patches/unwind")"
		fi
	} | jsc_sha256
}

jsc_sysroot_path() {
	local root=$1
	local target=${2:-wasm32-unknown-unknown}
	local dylib=$3
	local key
	key=$(jsc_sysroot_key "$root" "$target" "$dylib") || return
	printf '%s/build/sysroots/%s\n' "$root" "$key"
}

# Resolve the same cache for the compiler and its fixture harness. Only a default
# invocation can consume a pre-keyed checkout's real (not symlinked) legacy sysroot.
jsc_resolve_sysroot() {
	local root=$1
	local target=$2
	local dylib=$3
	local selected
	selected=$(jsc_sysroot_path "$root" "$target" "$dylib") || return
	if [ ! -d "$selected/lib/rustlib/$target/lib" ] && \
		[ -z "$(jsc_sysroot_args)" ] && [ -z "${JSC_RUST_OPT_LEVEL:-}" ] && \
		[ ! -L "$root/build/sysroot" ] && \
		[ -d "$root/build/sysroot/lib/rustlib/$target/lib" ]; then
		selected="$root/build/sysroot"
	fi
	printf '%s\n' "$selected"
}

# Replaces a compatibility symlink without following a directory symlink at the destination.
# Callers serialize writers before using this helper.
jsc_replace_symlink() {
	local source=$1
	local destination=$2
	if [ -e "$destination" ] && [ ! -L "$destination" ]; then
		echo "sysroot-cache.sh: refusing to replace non-symlink $destination" >&2
		return 1
	fi
	rm -f "$destination"
	mv "$source" "$destination"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
	if [ "$#" -ne 1 ]; then
		echo "usage: sysroot-cache.sh args|path" >&2
		exit 2
	fi
	if [ "$1" = "args" ]; then
		jsc_sysroot_args
		exit
	fi
	if [ "$1" != "path" ]; then
		echo "usage: sysroot-cache.sh args|path" >&2
		exit 2
	fi
	script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
	root=$(cd -- "$script_dir/.." && pwd)
	jsc_select_mode "$root" || exit 2
	target=${JSC_TARGET:-$jsc_target_name}
	case "$(uname -s)" in
	Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
	*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
	esac
	jsc_sysroot_path "$root" "$target" "$dylib"
fi
