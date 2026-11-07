#!/usr/bin/env bash
#
# Runs the runtime suite under every emission flag mode, and reports one table.
#
#     ./scripts/matrix.sh
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
# `mires-build` staleness check refuses a client build whose sysroot disagrees
# with the flags it wants. A matrix run leaves the sysroot at whatever mode ran
# last, so this script ALWAYS finishes by restoring the default. Wave 5 hit this
# from the other side -- a client build failed with `found "js-names=mangled",
# wanted ""` -- and the fix was to re-run the default by hand, which is a step
# nobody should have to remember.
#
# # `js-names=mangled`
#
# Included and expected to FAIL, at a pinned count. It has been broken since
# wave 3 and is listed in the OUT list; running it anyway is what keeps "broken"
# an observation with a number attached rather than a habit.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)
root=$(dirname "$script_dir")

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
)

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
expected_default=""
expected_names_mangled="36"

printf '%s\n' "==> flag-mode matrix, ${#modes[@]} modes"

results=()
failures=0
for entry in "${modes[@]}"; do
	name=${entry%%:*}
	flags=${entry#*:}
	printf '\n==> mode %s (%s)\n' "$name" "${flags:-<default>}"

	log="$root/build/logs/matrix-$name.log"
	if [ -z "$flags" ]; then
		JS_EXTRA_ARGS='' "$script_dir/test.sh" >"$log" 2>&1
	else
		JS_EXTRA_ARGS="$flags" "$script_dir/test.sh" >"$log" 2>&1
	fi
	status=$?

	line=$(grep -Eo '[0-9]+/[0-9]+ passed' "$log" | tail -1)
	passed=${line%%/*}
	total=$(printf '%s' "$line" | sed 's|.*/||; s| passed||')

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
printf '%-16s %-8s %-16s\n' "MODE" "RESULT" "COUNT"
printf -- '----------------------------------------------------------------\n'
for row in "${results[@]}"; do
	IFS='|' read -r name verdict count _ <<<"$row"
	printf '%-16s %-8s %-16s\n' "$name" "$verdict" "$count"
done
printf -- '----------------------------------------------------------------\n'
if [ "$restore" -ne 0 ]; then
	printf 'the sysroot restore run FAILED; build/sysroot/.js-args may be stale\n'
	failures=$((failures + 1))
fi
printf '%s\n' "$((${#modes[@]} - failures))/${#modes[@]} modes as expected"
printf '================================================================\n'

exit $((failures == 0 ? 0 : 1))
