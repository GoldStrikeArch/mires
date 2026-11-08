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

jsc_sysroot_args() {
	if [ "${JSC_SYSROOT_ARGS+x}" = x ]; then
		jsc_normalize_emit_args "$JSC_SYSROOT_ARGS"
	else
		jsc_normalize_emit_args "${JS_EXTRA_ARGS:-}"
	fi
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

jsc_patches_digest() {
	local root=$1
	local found=0
	local patch
	{
		for patch in "$root"/patches/*.patch; do
			[ -f "$patch" ] || continue
			found=1
			printf '%s %s\n' "$(basename -- "$patch")" "$(jsc_sha256_file "$patch")"
		done
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

	if [ ! -f "$dylib" ]; then
		echo "sysroot-cache.sh: backend not built: $dylib" >&2
		return 1
	fi
	channel=$(jsc_pinned_channel "$root")
	args=$(jsc_sysroot_args)
	if ! rustc_identity=$(rustc "+$channel" -vV 2>/dev/null); then
		echo "sysroot-cache.sh: cannot identify pinned toolchain $channel" >&2
		return 1
	fi

	{
		printf 'cache-format=1\n'
		printf 'backend=%s\n' "$(jsc_sha256_file "$dylib")"
		printf 'patches=%s\n' "$(jsc_patches_digest "$root")"
		printf 'channel=%s\n' "$channel"
		printf 'rustc=%s\n' "$rustc_identity"
		printf 'target=%s\n' "$target"
		printf 'rust-opt=%s\n' "$rust_opt"
		printf 'js-args=%s\n' "$args"
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
	target=${JSC_TARGET:-wasm32-unknown-unknown}
	case "$(uname -s)" in
	Darwin) dylib="$root/target/release/librustc_codegen_mires.dylib" ;;
	*) dylib="$root/target/release/librustc_codegen_mires.so" ;;
	esac
	jsc_sysroot_path "$root" "$target" "$dylib"
fi
