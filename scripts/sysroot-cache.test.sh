#!/usr/bin/env bash

set -euo pipefail
# The checks below describe the abort mode's keys (selected explicitly with `JSC_PANIC=abort`); the
# default unwind mode's keys (unset / `JSC_PANIC=unwind`) are checked at the end.
export JSC_PANIC=abort

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=sysroot-cache.sh
source "$script_dir/sysroot-cache.sh"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/root/target/release" "$work/bin"
printf 'channel = "nightly-test"\n' >"$work/root/rust-toolchain.toml"
printf 'backend-a' >"$work/root/target/release/backend"
mkdir -p "$work/root/patches"
printf 'patch-a' >"$work/root/patches/0001.patch"
cat >"$work/bin/rustc" <<'EOF'
#!/usr/bin/env bash
printf 'rustc 1.99.0\ncommit-hash: %s\nhost: test-host\n' "${FAKE_RUSTC_COMMIT:-aaa}"
EOF
chmod +x "$work/bin/rustc"
export PATH="$work/bin:$PATH"

key() {
	jsc_sysroot_key "$work/root" "${JSC_TARGET:-wasm32-unknown-unknown}" \
		"$work/root/target/release/backend"
}

first=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' key)
reordered=$(JS_EXTRA_ARGS='  js-minify=on   js-opt=report ' key)
[ "$first" = "$reordered" ]

off=$(JS_EXTRA_ARGS='js-opt=off js-minify=on' key)
on=$(JS_EXTRA_ARGS='js-opt=on js-minify=on' key)
[ "$off" != "$first" ]
[ "$on" != "$first" ]
[ "$off" != "$on" ]

last_on=$(JS_EXTRA_ARGS='js-minify=on js-opt=off js-opt=on' key)
last_off=$(JS_EXTRA_ARGS='js-minify=on js-opt=on js-opt=off' key)
[ "$last_on" = "$on" ]
[ "$last_off" = "$off" ]

ast_dce=$(JS_EXTRA_ARGS='js-opt=on js-opt-passes=ast-dce' key)
future_poller=$(JS_EXTRA_ARGS='js-opt=on js-opt-passes=future-poller' key)
both=$(JS_EXTRA_ARGS='js-opt=on js-opt-passes=ast-dce,future-poller' key)
both_reordered=$(JS_EXTRA_ARGS='js-opt-passes=future-poller,ast-dce js-opt=on' key)
native_representations=$(JS_EXTRA_ARGS='js-opt=on js-opt-passes=native-representations' key)
link_dedup=$(JS_EXTRA_ARGS='js-opt=on js-opt-passes=link-dedup' key)
sroa=$(JS_EXTRA_ARGS='js-opt=on js-opt-passes=sroa' key)
all=$(JS_EXTRA_ARGS='js-opt=on js-opt-passes=sroa,ast-dce,future-poller,native-representations,link-dedup' key)
all_reordered=$(JS_EXTRA_ARGS='js-opt-passes=link-dedup,native-representations,future-poller,ast-dce,sroa js-opt=on' key)
all_explicit=$(JS_EXTRA_ARGS='js-opt-passes=all js-opt=on' key)
all_default=$(JS_EXTRA_ARGS='js-opt=on' key)
almost_all=$(JS_EXTRA_ARGS='js-opt=on js-opt-passes=sroa,ast-dce,future-poller,native-representations' key)
[ "$ast_dce" != "$future_poller" ]
[ "$ast_dce" != "$both" ]
[ "$native_representations" != "$both" ]
[ "$link_dedup" != "$native_representations" ]
[ "$sroa" != "$ast_dce" ]
[ "$almost_all" != "$all" ]
[ "$both" = "$both_reordered" ]
[ "$all" = "$all_reordered" ]
[ "$all" = "$all_explicit" ]
[ "$all" = "$all_default" ]

opt_one=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' JSC_RUST_OPT_LEVEL=1 key)
[ "$opt_one" != "$first" ]

printf 'backend-b' >"$work/root/target/release/backend"
backend_b=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' key)
[ "$backend_b" != "$first" ]

printf 'patch-b' >"$work/root/patches/0001.patch"
patch_b=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' key)
[ "$patch_b" != "$first" ]
[ "$patch_b" != "$backend_b" ]
printf 'patch-a' >"$work/root/patches/0001.patch"

printf 'backend-a' >"$work/root/target/release/backend"
rustc_b=$(FAKE_RUSTC_COMMIT=bbb JS_EXTRA_ARGS='js-opt=report js-minify=on' key)
[ "$rustc_b" != "$first" ]

target_b=$(JSC_TARGET=test-target JS_EXTRA_ARGS='js-opt=report js-minify=on' key)
[ "$target_b" != "$first" ]

selected=$(JSC_SYSROOT_ARGS='js-opt=report js-minify=on' JS_EXTRA_ARGS='js-comments=off' key)
[ "$selected" = "$first" ]
selected_profile=$(JSC_SYSROOT_ARGS='js-profile=release-balanced js-opt=on' key)
[ "$selected_profile" = "$(JSC_SYSROOT_ARGS='js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=on' key)" ]

link_only=$(JS_EXTRA_ARGS='js-comments=off js-source-map=on js-modules=esm' key)
baseline=$(JS_EXTRA_ARGS='' key)
[ "$link_only" = "$baseline" ]

emit_with_link=$(JS_EXTRA_ARGS='js-comments=off js-opt=report js-minify=on' key)
[ "$emit_with_link" = "$first" ]

# `js-jir` decides what an object file contains, so it is emit-affecting and joins the key.
jir_off=$(JS_EXTRA_ARGS='js-jir=off js-minify=on' key)
jir_on=$(JS_EXTRA_ARGS='js-jir=on js-minify=on' key)
minify_only=$(JS_EXTRA_ARGS='js-minify=on' key)
[ "$jir_off" != "$minify_only" ]
[ "$jir_on" != "$jir_off" ]

cleanup_on=$(JS_EXTRA_ARGS='js-cleanup=on js-minify=on' key)
cleanup_off=$(JS_EXTRA_ARGS='js-cleanup=off js-minify=on' key)
[ "$cleanup_on" != "$cleanup_off" ]
[ "$cleanup_on" != "$minify_only" ]
[ "$cleanup_on" = "$(JS_EXTRA_ARGS='js-minify=on js-cleanup=on' key)" ]

# The lane options are link-only: both lanes travel in every object, so changing one must not
# cost a sysroot rebuild.
lane_jir=$(JS_EXTRA_ARGS='js-minify=on js-lane=jir' key)
lane_shadow=$(JS_EXTRA_ARGS='js-minify=on js-lane=shadow js-lane-report=/tmp/lane.json' key)
lane_conflicts=$(JS_EXTRA_ARGS='js-minify=on js-jir-dump-conflicts=/tmp/c.json' key)
[ "$lane_jir" = "$minify_only" ]
[ "$lane_shadow" = "$minify_only" ]
[ "$lane_conflicts" = "$minify_only" ]

# `js-opt-weight-alloc` tunes the link-time representation cost model only (Phase 4), so it
# stays out of the key too: retuning the weight must never cost a sysroot rebuild.
weight_alloc=$(JS_EXTRA_ARGS='js-minify=on js-opt-weight-alloc=40' key)
[ "$weight_alloc" = "$minify_only" ]

# A profile expands to its emit-affecting components, so it shares a key with the spelled out
# options and never builds a second identical sysroot.
profile_size=$(JS_EXTRA_ARGS='js-profile=release-size' key)
expanded_size=$(JS_EXTRA_ARGS='js-cleanup=on js-ecma-target=2018 js-lane=jir js-minify=on js-opt=on' key)
[ "$profile_size" = "$expanded_size" ]

profile_balanced=$(JS_EXTRA_ARGS='js-profile=release-balanced' key)
[ "$profile_balanced" = "$profile_size" ]

profile_speed=$(JS_EXTRA_ARGS='js-profile=release-speed' key)
expanded_speed=$(JS_EXTRA_ARGS='js-cleanup=on js-ecma-target=2018 js-lane=jir js-minify=locals js-opt=on' key)
[ "$profile_speed" = "$expanded_speed" ]
[ "$profile_speed" != "$profile_size" ]

profile_dev=$(JS_EXTRA_ARGS='js-profile=dev' key)
expanded_dev=$(JS_EXTRA_ARGS='js-opt=off js-lane=baseline' key)
[ "$profile_dev" = "$expanded_dev" ]

profile_report=$(JS_EXTRA_ARGS='js-profile=report' key)
expanded_report=$(JS_EXTRA_ARGS='js-opt=report' key)
[ "$profile_report" = "$expanded_report" ]
[ "$profile_report" != "$profile_dev" ]

# An option spelled out alongside a profile overrides it, on either side of it, exactly as the
# backend parses it.
profile_then_flag=$(JS_EXTRA_ARGS='js-profile=release-size js-minify=off' key)
flag_then_profile=$(JS_EXTRA_ARGS='js-minify=off js-profile=release-size' key)
[ "$profile_then_flag" = "$flag_then_profile" ]
[ "$profile_then_flag" != "$profile_size" ]
[ "$profile_then_flag" = "$(JS_EXTRA_ARGS='js-cleanup=on js-ecma-target=2018 js-minify=off js-opt=on' key)" ]

# The syntax target changes item text and therefore owns a distinct sysroot. An explicit latest
# target can override the ES2018 release-profile default on either side of the alias.
target_2018=$(JS_EXTRA_ARGS='js-ecma-target=2018' key)
[ "$target_2018" != "$baseline" ]
profile_then_target=$(JS_EXTRA_ARGS='js-profile=release-size js-ecma-target=latest' key)
target_then_profile=$(JS_EXTRA_ARGS='js-ecma-target=latest js-profile=release-size' key)
[ "$profile_then_target" = "$target_then_profile" ]
[ "$profile_then_target" = "$(JS_EXTRA_ARGS='js-cleanup=on js-ecma-target=latest js-minify=on js-opt=on' key)" ]

# Profiles are overlaid in order, so a later one wins for every option it names and leaves the
# options it does not name where the earlier one put them. That is what the backend does too.
last_profile=$(JS_EXTRA_ARGS='js-profile=dev js-profile=report' key)
[ "$last_profile" = "$profile_report" ]
overlaid=$(JS_EXTRA_ARGS='js-profile=release-size js-profile=report' key)
[ "$overlaid" = "$(JS_EXTRA_ARGS='js-cleanup=on js-ecma-target=2018 js-minify=on js-opt=report' key)" ]

# A misspelled profile keeps a key of its own rather than quietly sharing the default's.
bad_profile=$(JS_EXTRA_ARGS='js-profile=release' key)
[ "$bad_profile" != "$baseline" ]

path=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' \
	jsc_sysroot_path "$work/root" wasm32-unknown-unknown "$work/root/target/release/backend")
[ "$path" = "$work/root/build/sysroots/$first" ]

resolve_sysroot() {
	jsc_resolve_sysroot "$work/root" wasm32-unknown-unknown "$work/root/target/release/backend"
}
mkdir -p "$path/lib/rustlib/wasm32-unknown-unknown/lib"
[ "$(JS_EXTRA_ARGS='js-opt=report js-minify=on' resolve_sysroot)" = "$path" ]
[ ! -e "$work/root/build/sysroot" ]
mkdir -p "$work/root/build/sysroot/lib/rustlib/wasm32-unknown-unknown/lib"
[ "$(JS_EXTRA_ARGS='' resolve_sysroot)" = "$work/root/build/sysroot" ]
[ "$(JS_EXTRA_ARGS='js-opt=on' resolve_sysroot)" != "$work/root/build/sysroot" ]
[ "$(JS_EXTRA_ARGS='' JSC_RUST_OPT_LEVEL=1 resolve_sysroot)" != "$work/root/build/sysroot" ]
mv "$work/root/build/sysroot" "$work/root/build/old-default"
ln -s "$path" "$work/root/build/sysroot"
[ "$(JS_EXTRA_ARGS='' resolve_sysroot)" != "$work/root/build/sysroot" ]

mkdir -p "$work/links/old" "$work/links/new"
ln -s old "$work/links/current"
ln -s new "$work/links/replacement"
jsc_replace_symlink "$work/links/replacement" "$work/links/current"
[ "$(readlink "$work/links/current")" = new ]
[ ! -e "$work/links/old/replacement" ]

# The compiler is two programs, and only one of them decides what an object file contains. The
# key digests the backend dylib alone, so editing `jsc-link` -- the whole linker and optimizer --
# leaves every cached sysroot valid, and editing the backend invalidates them. That is the property
# the producer/consumer split exists for; `scripts/build.sh` builds both binaries into the same
# directory, which is what makes it worth pinning here.
printf 'linker-a' >"$work/root/target/release/jsc-link"
with_linker_a=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' key)
[ "$with_linker_a" = "$first" ]
printf 'linker-b' >"$work/root/target/release/jsc-link"
with_linker_b=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' key)
[ "$with_linker_b" = "$first" ]

printf 'backend-b' >"$work/root/target/release/backend"
with_backend_b=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' key)
[ "$with_backend_b" != "$first" ]
printf 'backend-a' >"$work/root/target/release/backend"

printf 'sysroot cache key checks passed\n'

# Explicit opt-out must have a distinct key and be independent of argument order.
cleanup_off=$(JS_EXTRA_ARGS="js-profile=release-balanced js-cleanup=off" key)
[ "$cleanup_off" != "$profile_balanced" ]
[ "$cleanup_off" = "$(JS_EXTRA_ARGS="js-cleanup=off js-profile=release-balanced" key)" ]

# The unwind build mode (unset or `JSC_PANIC=unwind`, the default) is keyed apart from every
# abort sysroot (`JSC_PANIC=abort`): its panic strategy, its target and the target spec's content
# join the key, and `js-cleanup=off` follows the caller's options, so a pinned `js-cleanup=on` or
# a release profile is overridden. `abort` is the opt-in override, and anything else is refused.
unset JSC_PANIC
mkdir -p "$work/root/targets"
printf 'spec-a' >"$work/root/targets/mires32-unwind.json"
unwind_key() { JSC_TARGET=mires32-unwind key; }
[ "$(JSC_PANIC=abort JS_EXTRA_ARGS='js-opt=report js-minify=on' key)" = "$first" ]
unwind=$(JS_EXTRA_ARGS='js-opt=report js-minify=on' unwind_key)
[ "$unwind" = "$(JSC_PANIC=unwind JS_EXTRA_ARGS='js-opt=report js-minify=on' unwind_key)" ]
[ "$unwind" != "$first" ]
[ "$unwind" != "$(JSC_PANIC=unwind JS_EXTRA_ARGS='js-opt=report js-minify=on' key)" ]
[ "$unwind" = "$(JS_EXTRA_ARGS='js-opt=report js-minify=on js-cleanup=off' unwind_key)" ]
[ "$(JS_EXTRA_ARGS='js-profile=release-balanced js-cleanup=on' jsc_sysroot_args)" = \
	'js-cleanup=off js-ecma-target=2018 js-minify=on js-opt=on' ]
[ "$(JSC_PANIC=unwind JS_EXTRA_ARGS='js-profile=release-balanced js-cleanup=on' jsc_sysroot_args)" = \
	'js-cleanup=off js-ecma-target=2018 js-minify=on js-opt=on' ]
[ "$(JSC_SYSROOT_ARGS='js-cleanup=on' JS_EXTRA_ARGS='js-opt=on' jsc_sysroot_args)" = 'js-cleanup=off' ]
[ "$(JS_EXTRA_ARGS='' jsc_sysroot_args)" = 'js-cleanup=off' ]
[ "$(jsc_mode_backend_args 'js-profile=release-balanced js-lane=jir')" = \
	'js-profile=release-balanced js-lane=jir js-cleanup=off' ]
[ "$(JSC_PANIC=abort jsc_mode_backend_args '  js-opt=on ')" = '  js-opt=on ' ]
printf 'spec-b' >"$work/root/targets/mires32-unwind.json"
[ "$(JS_EXTRA_ARGS='js-opt=report js-minify=on' unwind_key)" != "$unwind" ]
[ "$(JSC_PANIC=abort JS_EXTRA_ARGS='js-opt=report js-minify=on' key)" = "$first" ]
rm "$work/root/targets/mires32-unwind.json"
refused() { if ("$@") >/dev/null 2>&1; then echo "not refused: $*" >&2; exit 1; fi; }
refused unwind_key
JSC_PANIC=unwind refused unwind_key
refused env JSC_PANIC=Unwind bash -c "source '$script_dir/sysroot-cache.sh' && jsc_sysroot_args"
JSC_PANIC=Unwind refused key
printf 'unwind mode cache key checks passed\n'
