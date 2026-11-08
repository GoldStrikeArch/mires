#!/usr/bin/env bash
#
# Runs the runtime suite under every emission flag mode, and reports one table.
#
#     ./scripts/matrix.sh              # every mode
#     ./scripts/matrix.sh @passes      # just the named mode(s); the @ is optional
#
# `G2-CHECKLIST.md`'s last structural finding is that this did not exist: "The 9
# flag modes are not encoded anywhere. There is no CI file, no Makefile and no
# matrix script in the spike ... A verdict claiming '46/46 across 9 modes' is
# claiming the result of nine manual invocations. Either write the matrix script
# or say it is manual." This is the script, so the claim is now a command
# somebody can run.
#
# # Why the modes are what they are
#
# One mode per INDEPENDENT decision the emitter makes, plus the default. Each
# changes the shape of the emitted JavaScript without changing what it means, so
# every one of them has to produce a program that behaves identically. A mode
# that only changes comments is still here: a comment writer that runs off the
# end of a statement run is a real defect, and it is invisible in any other mode.
#
# # The sysroot, and why the last line of this script matters
#
# `build/sysroot/.js-args` records the flags the sysroot was built with, and the
# framework's staleness check refuses a demo-app build whose sysroot disagrees
# with the flags it wants. A matrix run leaves the sysroot at whatever mode ran
# last, so this script ALWAYS finishes by restoring the default. Wave 5 hit this
# from the other side -- a demo-app build failed with `found "js-names=mangled",
# wanted ""` -- and the fix was to re-run the default by hand, which is a step
# nobody should have to remember.
#
# # `js-names=mangled`
#
# Included and expected to FAIL, at a pinned count. It has been broken since
# wave 3 and is listed in the OUT list; running it anyway is what keeps "broken"
# an observation with a number attached rather than a habit.
#
# # The lane modes
#
# `lane-jir` runs the suite with `-Cllvm-args=js-lane=jir`: every reachable item
# is offered to the JIR lowerer and the ones it declines keep v1's printed text.
# It is expected to pass in full, and today it passes with EVERY item falling
# back, which is the point rather than a caveat -- total fallback has to be a
# working compiler before partial fallback can be trusted. `js-lane` is not
# emit-affecting, so the mode links against the sysroot that is already there.
#
# `lane-jir-minify` and `lane-jir-minify-locals` are the two minifier modes on
# the JIR lane rather than the baseline, because minification is the one thing
# that reads an item's printed text after a lane produced it: `shorten_locals`
# runs on the lowered AST in the link step (`link/lane.rs`) and `item_renames`
# runs over the whole printed corpus. They share the minify sysroots with the
# baseline modes above, since `js-lane` is not emit-affecting.
#
# `lane-jir-opt` and `lane-jir-opt-minify` are the JIR lane with the optimizer
# ON. Through Phase 2 that combination was refused outright -- v1's optimizer
# rewrites PRINTED baseline JavaScript that the lane does not print -- and Phase
# 3 lifted the refusal by re-landing the rewrites as JIR passes
# (jsc-link/src/opt/). These two modes are the metamorphic check on that: the
# suite's `.expected` files are the same files the `off` modes are checked
# against, so a pass that changes what a program MEANS fails here.
#
# `js-opt` IS emit-affecting, so unlike the other lane modes these two build
# their own sysroot. test.sh notices the mismatch and rebuilds it (about a
# minute, once, then cached under build/sysroots/<key>).
#
# `lane-suites-opt` is not a test.sh mode either: it runs the four suites that
# pin their own flags -- emit, module, dts, extern -- on the JIR lane with each
# one's own `*_OPT_MODE=on`. They cannot be driven from $JS_EXTRA_ARGS (a golden
# suite that inherited the caller's flags would not have goldens), which is
# exactly why `js-opt` used to be dropped on the floor there; this mode is what
# stops that from being invisible again. The counts of the four are summed.
#
# `lane-shadow` is not a test.sh mode at all: it runs
# scripts/lane-shadow-test.sh, which compiles each fixture ONCE with
# `js-lane=shadow` and runs both printed programs. That is the differential the
# other modes cannot express -- they compare a program against an expectation,
# and this compares two lanes of one compilation against each other.
# `lane-shadow-opt` runs the same differential with the JIR optimizer on. It is a separate row so
# the default shadow configuration cannot hide an optimizer-only wrong-code regression.
#
# `passes` is the metamorphic check on the JIR pass pipeline (Phase 5, D5):
# `RCGJS_JIR_PASSES` is the link-time env that selects which optimizer passes
# run (unset = all), and this mode runs the full suite once per leave-one-out
# subset. Every subset is checked against the same `.expected` files, so "an
# optimizer pass never changes what a program means" holds for each subset, not
# just for the full pipeline. The env is consumed at link time -- it is not
# emit-affecting, so every subset shares the one `js-opt=on` sysroot.
#
# `checks` is the general hook for bespoke pass/fail scripts (phase-3 note 7's
# wish): each one asserts a fallback or decline path a golden suite cannot --
# the flat-ABI retry, the representation commit rollback, the custom-allocator
# decline and mixed-call-site versioning -- and reports PASS/FAIL by exit code, summed into one
# row.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
root=$(dirname "$script_dir")
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"
mkdir -p "$root/build/logs"
: "${RCGJS_WITNESS_CHECK:=1}"
export RCGJS_WITNESS_CHECK
# A matrix row claiming the runtime suite must never be the 19-test mini-core subset.
REQUIRE_CORE=1
export REQUIRE_CORE

# name:flags. An empty flag string is the default mode.
modes=(
	"default:"
	"trampoline:js-structure=trampoline"
	"queue-off:js-queue=off"
	"switch-flat:js-switch=flat"
	"scoped-lets:js-scoped-lets"
	"line-comments:js-line-comments"
	"source-map:js-source-map=on"
	"minify:js-minify=on"
	"minify-locals:js-minify=locals"
	"names-mangled:js-names=mangled"
	"lane-jir:js-lane=jir"
	"lane-jir-minify:js-minify=on js-lane=jir"
	"lane-jir-minify-locals:js-minify=locals js-lane=jir"
	"lane-jir-opt:js-opt=on js-lane=jir"
	"lane-jir-opt-minify:js-opt=on js-minify=on js-lane=jir"
	"lane-shadow:@shadow"
	"lane-shadow-opt:@shadow-opt"
	"lane-suites-opt:@suites-opt"
	"passes:@passes"
	"checks:@checks"
)

# With arguments, only the named modes run (the leading @ is optional, so the
# spelling `matrix.sh @passes` works). Without any, every mode does.
if [ "$#" -gt 0 ]; then
	selected=()
	for want in "$@"; do
		want=${want#@}
		found=0
		for entry in "${modes[@]}"; do
			if [ "${entry%%:*}" = "$want" ]; then
				selected+=("$entry")
				found=1
				break
			fi
		done
		if [ "$found" -eq 0 ]; then
			echo "matrix.sh: unknown mode: $want" >&2
			echo "matrix.sh: available:" "${modes[@]%%:*}" >&2
			exit 2
		fi
	done
	modes=("${selected[@]}")
fi

# The count a mode is expected to reach. Every mode is the full suite except
# `names-mangled`, whose number is the pre-existing breakage this pins.
#
# The number moves whenever a test is ADDED, so a change to it has to name the
# tests that moved it. 33 -> 35: `28_array_into_iter` passes here outright, and
# `guard_map_key` passes because it is an `.expect_fail` test and never reaches
# codegen, while `30_hashmap` fails with the mode's own defect -- "two different
# definitions of ...", one `core` path spelled two ways across two crates --
# exactly as every other test that links `alloc` does. 35 -> 36: `guard_rc_str`
# passes for the `.expect_fail` reason, and `29_rc` fails for the `alloc` one.
# 36 -> 42: re-pinned after the pin went stale sometime before the 2026-08-14
# optimization round (measured 42 both before and after that round's changes).
# The six are the fixtures added since the pin was written that avoid `alloc`
# linkage or are `.expect_fail` guards, so they pass under mangled names too;
# every failure in the mode remains the pre-existing two-crate `core` path
# spelling defect. 42 -> 43: `54_str_index_no_fmt` avoids `alloc` and passes;
# `55_native_host_string`, `56_native_format_args`, and `57_native_format_sinks`
# link `alloc` and fail with the same standing spelling defect.
#
# 43 -> 75: the standing defect is FIXED. Phase 1 W4 replaced the link step's
# text-equality duplicate check with the JIR identity rule (`SemanticKey` plus
# the normalized body hash), so one `core` item spelled two ways across two
# crates is now recognized as one item instead of raising "two different
# definitions of ...". Every fixture in the mode links. The eight that still
# fail -- `04_iter_range`, `12_slice_iter`, `13_sort`, `21_vec`, `26_async`,
# `30_hashmap`, `31_host`, `32_host_boundary` -- all fail `over-budget`, which
# is the mode's own cost (a mangled name is longer than a readable one) and not
# a link error. See `JIR.md`.
#
# 75 -> 82: seven fixtures were added in Phase 3 (`60_virtual_struct_merge`,
# `61_virtual_loop_carried`, `62_virtual_copy_semantics`, `63_virtual_enum_option`,
# `64_closure_env_virtual`, `65_flat_abi_spill`, `66_virtual_enum_copy`) and all
# seven avoid `alloc` linkage, so they pass under mangled names too. The eight
# that still fail are the same eight as before, all `over-budget`.
#
# 85 -> 91: six fixtures added in Phase 4 pass under mangled names too
# (`71_repr_string_builder`, `73_repr_commit_two_regions`, `74_repr_rollback`,
# `75_repr_native_vec`, `76_repr_vec_drop_order`, `77_pea_merge_materialize`),
# and `72_repr_format_min` links the `alloc` format machinery and fails
# `over-budget` -- the mode's own cost, joining the standing eight.
#
# 91 -> 93: the two post-review fixtures pass under mangled names too
# (`70_host_contract_slots`, `78_repr_capacity_edge`); the nine failures are
# `72` plus the standing eight, all `over-budget`.
#
# 93 -> 94: `guard_custom_allocator` (Phase 5, D10f) links `alloc` but carries
# no `.maxbytes`, so it passes under mangled names too; the nine failures are
# unchanged, all `over-budget`.
#
# 94 -> 97: three Phase 5 fixtures pass under mangled names too:
# `79_region_cut_vec` (W5a, links `alloc`, no `.maxbytes`),
# `80_switch_thread_variant` and `81_bounds_range` (W5c, both avoid `alloc`).
# The nine failures are unchanged, all `over-budget`.
#
# 97 -> 101: four Phase 5 wave-2 fixtures pass under mangled names too:
# `82_inline_next_loop`, `83_devirt_fnptr`, and `84_inline_budget` (W5e) avoid
# `alloc` linkage, and `85_raw_view_slice` (W5f) carries no `.maxbytes`. The
# nine failures are unchanged, all `over-budget`.
#
# 101 -> 102: `86_payload_split_call` (Phase 5 wave 3) avoids `alloc` linkage and
# passes under mangled names too. The nine failures are unchanged, all `over-budget`.
#
# 102 -> 103: `87_repr_private_version` (D4 closure) carries no `.maxbytes` and
# passes under mangled names too. The nine failures are unchanged, all `over-budget`.
#
# 103 -> 104: `88_repr_mixed_callsite_version` pins the mixed native/baseline private-version
# rollback. It carries no `.maxbytes`, so it passes under mangled names too.
#
# 104 -> 108: the four remediation fixtures all pass under mangled names:
# `89_repr_box_drop`, `90_repr_box_structural_drop`, `91_flat_result`, and
# `92_projection_mutation`. The nine standing `over-budget` failures are unchanged.
#
# 108 -> 113: the product-wrapper and physicalization hardening fixtures 93–97 all carry no
# `.maxbytes`, so they pass under mangled names. The nine standing failures are unchanged.
#
# 113 -> 114: `98_product_lanes` carries no `.maxbytes` and passes under mangled names. The nine
# standing failures are unchanged.
#
# 114 -> 115: `99_recursive_adt_iter` carries no `.maxbytes` and passes under mangled names. The
# nine standing failures are unchanged.
#
# 115 -> 119: fixtures 100, 101, 103, and 104 carry no `.maxbytes` and pass under mangled names.
# `102_str_lines` has a byte budget and joins the nine standing over-budget failures, for ten total.
# With typed storage disabled by default, fixture 104 follows the ordinary native-Vec path.
expected_default=""
expected_names_mangled="119"

# The four suites that pin their own compile flags, each with the optimizer on, on the JIR lane.
# One line of `<passed>/<total> passed` at the end, summed, so the table reads like every other row.
suites_opt() {
	local passed=0 total=0 status=0 suite line
	for suite in emit module dts extern; do
		local var
		var=$(printf '%s_OPT_MODE' "$(printf '%s' "$suite" | tr '[:lower:]' '[:upper:]')")
		printf '\n==> %s-test.sh (%s=on, js-lane=jir)\n' "$suite" "$var"
		if ! env "$var=on" JS_EXTRA_ARGS='js-lane=jir' SKIP_BUILD=1 \
			"$script_dir/$suite-test.sh"; then
			status=1
		fi >"$root/build/logs/matrix-suites-opt-$suite.log" 2>&1
		cat "$root/build/logs/matrix-suites-opt-$suite.log"
		line=$(grep -Eo '[0-9]+/[0-9]+ passed' "$root/build/logs/matrix-suites-opt-$suite.log" |
			tail -1)
		passed=$((passed + ${line%%/*}))
		total=$((total + $(printf '%s' "$line" | sed 's|.*/||; s| passed||')))
	done
	printf '\n%d/%d passed\n' "$passed" "$total"
	return "$status"
}

# The registered JIR pass names, compiler execution order (D5). `RCGJS_JIR_PASSES` names the
# selection at link time; a name in this list nothing consumes yet is a subset that behaves like
# the full pipeline, which is the harness landing before the passes it will one day catch.
jir_passes="constprop inline devirt pea slice-iter range dce"

# One full suite run per leave-one-out subset, all against the same goldens.
passes_loo() {
	local passed=0 total=0 status=0 leave pass subset line
	for leave in $jir_passes; do
		subset=""
		for pass in $jir_passes; do
			[ "$pass" = "$leave" ] || subset="$subset${subset:+,}$pass"
		done
		printf '\n==> test.sh (js-lane=jir js-opt=on, RCGJS_JIR_PASSES=%s)\n' "$subset"
		if ! RCGJS_JIR_PASSES="$subset" JS_EXTRA_ARGS='js-lane=jir js-opt=on' SKIP_BUILD=1 \
			"$script_dir/test.sh" >"$root/build/logs/matrix-passes-no-$leave.log" 2>&1; then
			status=1
		fi
		line=$(grep -Eo '[0-9]+/[0-9]+ passed' "$root/build/logs/matrix-passes-no-$leave.log" |
			tail -1)
		printf '%s\n' "without $leave: ${line:-no result}"
		passed=$((passed + ${line%%/*}))
		total=$((total + $(printf '%s' "$line" | sed 's|.*/||; s| passed||')))
	done
	printf '\n%d/%d passed\n' "$passed" "$total"
	return "$status"
}

# The pass/fail receipt scripts, one PASS each by exit code.
check_scripts() {
	local passed=0 total=0 status=0 script
	printf '\n==> building the backend used by bespoke checks\n'
	if ! "$script_dir/build.sh" >"$root/build/logs/matrix-check-build.log" 2>&1; then
		cat "$root/build/logs/matrix-check-build.log"
		printf '\n0/22 passed\n'
		return 1
	fi
	for script in abi-retry-test repr-rollback-test allocator-guard-test repr-sourcemap-test macro-sourcemap-test island-sourcemap-test native-clone-test mixed-version-test native-box-test carrier-pea-test flat-result-test slice-lane-test slice-iterator-test projection-mutation-test sroa-test for-handler-test cross-crate-physical-test storage-plan-test async-graph-test async-action-test es2018-test operand-effects-test artifact-manifest-test keyed-compiled-test host-operations-test cross-object-inline-test operation-census-test; do
		total=$((total + 1))
		printf '\n==> %s.sh\n' "$script"
		if SKIP_BUILD=1 "$script_dir/$script.sh" \
			>"$root/build/logs/matrix-check-$script.log" 2>&1; then
			passed=$((passed + 1))
		else
			status=1
		fi
		cat "$root/build/logs/matrix-check-$script.log"
	done
	printf '\n%d/%d passed\n' "$passed" "$total"
	return "$status"
}

printf '%s\n' "==> flag-mode matrix, ${#modes[@]} modes"

# `test.sh` deliberately permits its real-core suite to be absent on a fresh checkout. The
# acceptance matrix does not: prepare the default keyed sysroot before its first row so a clean
# run cannot report a vacuous 19-test pass. Later rows let `test.sh` activate their keyed sysroots
# as before, and the restore at the end returns to this one.
for entry in "${modes[@]}"; do
	if [ "${entry%%:*}" = default ]; then
		printf '\n==> preparing the default sysroot\n'
		if ! JS_EXTRA_ARGS='' "$script_dir/build_sysroot.sh" \
			>"$root/build/logs/matrix-default-sysroot.log" 2>&1; then
			cat "$root/build/logs/matrix-default-sysroot.log" >&2
			exit 1
		fi
		break
	fi
done

results=()
failures=0
for entry in "${modes[@]}"; do
	name=${entry%%:*}
	flags=${entry#*:}
	printf '\n==> mode %s (%s)\n' "$name" "${flags:-<default>}"

	log="$root/build/logs/matrix-$name.log"
	if [ "$flags" = "@shadow" ]; then
		"$script_dir/lane-shadow-test.sh" >"$log" 2>&1
	elif [ "$flags" = "@shadow-opt" ]; then
		SHADOW_OPT_MODE=on "$script_dir/lane-shadow-test.sh" >"$log" 2>&1
	elif [ "$flags" = "@suites-opt" ]; then
		suites_opt >"$log" 2>&1
	elif [ "$flags" = "@passes" ]; then
		passes_loo >"$log" 2>&1
	elif [ "$flags" = "@checks" ]; then
		check_scripts >"$log" 2>&1
	elif [ -z "$flags" ]; then
		JS_EXTRA_ARGS='' "$script_dir/test.sh" >"$log" 2>&1
	else
		JS_EXTRA_ARGS="$flags" "$script_dir/test.sh" >"$log" 2>&1
	fi
	status=$?

	if [ "$flags" = "@shadow" ] || [ "$flags" = "@shadow-opt" ]; then
		line=$(grep -Eo '[0-9]+/[0-9]+ agree' "$log" | tail -1)
	elif [ "$flags" = "@suites-opt" ]; then
		line=$(grep -Eo '[0-9]+/[0-9]+ passed' "$log" | tail -1)
	else
		line=$(grep -Eo '[0-9]+/[0-9]+ passed' "$log" | tail -1)
	fi
	passed=${line%%/*}
	total=$(printf '%s' "$line" | sed 's|.*/||; s| passed||; s| agree||')

	verdict="PASS"
	if [ "$name" = "names-mangled" ]; then
		# Pinned breakage: the number is the assertion, not the exit code.
		if [ "$passed" != "$expected_names_mangled" ]; then
			verdict="CHANGED"
			failures=$((failures + 1))
		else
			verdict="PINNED"
		fi
	elif [ "$status" -ne 0 ] || [ "$passed" != "$total" ]; then
		verdict="FAIL"
		failures=$((failures + 1))
	fi

	results+=("$name|$verdict|${line:-no result}|$log")
done

# Always leave the sysroot on the default flags: see the header.
printf '\n==> restoring the sysroot to the default flags\n'
JS_EXTRA_ARGS='' "$script_dir/test.sh" >"$root/build/logs/matrix-restore.log" 2>&1
restore=$?

printf '\n================================================================\n'
printf '%-24s %-8s %-16s\n' "MODE" "RESULT" "COUNT"
printf -- '----------------------------------------------------------------\n'
for row in "${results[@]}"; do
	IFS='|' read -r name verdict count _ <<<"$row"
	printf '%-24s %-8s %-16s\n' "$name" "$verdict" "$count"
done
printf -- '----------------------------------------------------------------\n'
if [ "$restore" -ne 0 ]; then
	printf 'the sysroot restore run FAILED; build/sysroot/.js-args may be stale\n'
	failures=$((failures + 1))
fi
printf '%s\n' "$((${#modes[@]} - failures))/${#modes[@]} modes as expected"
printf '================================================================\n'

exit $((failures == 0 ? 0 : 1))
