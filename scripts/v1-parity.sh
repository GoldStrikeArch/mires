#!/usr/bin/env bash
#
# Checks that this checkout's baseline lane still emits exactly what the frozen v1
# compiler emits.
#
#   scripts/v1-parity.sh [--run] [v1-root]
#
# Compares every `build/*.js` and `build/core/*.js` produced by scripts/test.sh in
# this tree against the same file under the v1 tree (default: the sibling
# `rustc-codegen-js`). With `--run` it runs `scripts/test.sh` in both trees first;
# without it, both build directories must already be populated. The map files are
# compared too when both sides have them.
#
# Exit status is the number of files that differ or are missing (capped at 1 for
# the shell), so a green run is silent apart from the summary line.
#
# Scope: default `js-opt=off` output only. `js-opt=on` parity is out of scope —
# v2 deleted v1's link-time text optimizer (W5h), so the lanes' `on` builds are
# intentionally different programs.

set -uo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"

run=0
if [ "${1:-}" = "--run" ]; then
	run=1
	shift
fi
v1=${1:-$root/../rustc-codegen-js}
if [ ! -d "$v1" ]; then
	echo "v1-parity: no v1 checkout at $v1" >&2
	echo "v1-parity: pass its path explicitly: scripts/v1-parity.sh [--run] <v1-root>" >&2
	exit 2
fi
v1=$(cd -- "$v1" && pwd) || exit 2

if [ "$run" = 1 ]; then
	(cd "$v1" && scripts/test.sh) || echo "v1-parity: v1 test.sh reported failures (continuing with the diff)" >&2
	(cd "$root" && scripts/test.sh) || echo "v1-parity: v2 test.sh reported failures (continuing with the diff)" >&2
fi

require_artifacts() {
	local label=$1 tree=$2
	if [ ! -d "$tree/build/core" ] ||
		! find "$tree/build/core" -maxdepth 1 -type f -name '*.js' -print -quit | grep -q .; then
		echo "v1-parity: $label artifacts are absent under $tree/build/core" >&2
		echo "v1-parity: rerun with --run, or prepare both build trees first" >&2
		return 1
	fi
}

require_artifacts v1 "$v1" || exit 2
require_artifacts v2 "$root" || exit 2

same=0
differ=0
missing=0
expected_missing=0

# Fixtures added by v2 have no v1 artifact by construction. Keep the set explicit so a newly
# missing old artifact still fails instead of being waved through as "some v2 fixture".
v2_only='core/60_virtual_struct_merge.js
core/61_virtual_loop_carried.js
core/62_virtual_copy_semantics.js
core/63_virtual_enum_option.js
core/64_closure_env_virtual.js
core/65_flat_abi_spill.js
core/66_virtual_enum_copy.js
core/67_spill_name_collision.js
core/68_spill_name_collision_two.js
core/69_dead_call_dropped.js
core/70_host_contract_slots.js
core/71_repr_string_builder.js
core/72_repr_format_min.js
core/73_repr_commit_two_regions.js
core/74_repr_rollback.js
core/75_repr_native_vec.js
core/76_repr_vec_drop_order.js
core/77_pea_merge_materialize.js
core/78_repr_capacity_edge.js
core/79_region_cut_vec.js
core/80_switch_thread_variant.js
core/81_bounds_range.js
core/82_inline_next_loop.js
core/83_devirt_fnptr.js
core/84_inline_budget.js
core/85_raw_view_slice.js
core/86_payload_split_call.js
core/87_repr_private_version.js
core/88_repr_mixed_callsite_version.js
core/89_repr_box_drop.js
core/90_repr_box_structural_drop.js
core/91_flat_result.js
core/92_projection_mutation.js
core/guard_custom_allocator.js'

is_v2_only() {
	local base=${1%.map}
	grep -qxF -- "$base" <<<"$v2_only"
}

for sub in "" core; do
	dir="$root/build${sub:+/$sub}"
	[ -d "$dir" ] || continue
	for f in "$dir"/*.js "$dir"/*.js.map; do
		[ -e "$f" ] || continue
		rel=${f#"$root/build/"}
		other="$v1/build/$rel"
		if [ ! -e "$other" ]; then
			if is_v2_only "$rel"; then
				expected_missing=$((expected_missing + 1))
			else
				echo "unexpectedly missing in v1: $rel"
				missing=$((missing + 1))
			fi
		elif cmp -s "$f" "$other"; then
			same=$((same + 1))
		else
			echo "differs: $rel"
			differ=$((differ + 1))
		fi
	done
done

echo "v1-parity: $same identical, $differ differ, $missing unexpected missing, $expected_missing expected v2-only (v1 = $v1)"
[ $((differ + missing)) -eq 0 ]
