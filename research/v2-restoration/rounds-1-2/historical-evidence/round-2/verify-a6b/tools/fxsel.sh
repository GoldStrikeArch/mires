#!/usr/bin/env bash
# fxsel.sh: scripts/test.sh, one fixture at a time, under release-balanced + minify (the COMMON.md
# gate's options) with every link done by both linkers, for the fixtures the lean comparison
# (tools/lean-ab.sh) could not link because they need `--extern`, and for the fixtures this item
# is about (260 to 268). One row per fixture in gates/fxsel.tsv: the suite's verdict line and the
# link comparison.
set -uo pipefail
source "$(dirname "$0")/env.sh"
G=$V/gates
mkdir -p "$G"
: > "$G/fxsel.tsv"
for f in 30_hashmap 31_host 32_host_boundary 34_callback_ownership 55_native_host_string 70_host_contract_slots \
  guard_fallible_js_value guard_host_read guard_host_value guard_map_key guard_rc_str 113_unsized_tail \
  260_static_cycles 261_static_cycle_cells 262_chunk_owned_statics 263_chunk_three_owners 264_chunk_static_cycles \
  265_chunk_static_entries 266_chunk_static_entry_refusals 267_static_cycle_arrays 268_static_cycle_proto_key; do
  bash "$V/tools/abrun.sh" "fx-$f" env SKIP_BUILD=1 JS_EXTRA_ARGS='js-profile=release-balanced js-minify=on' bash scripts/test.sh "$f" > /dev/null 2>&1
  printf '%s\t%s\t%s\t%s\n' "$f" "$(grep -E "^core/$f +" "$V/ab2/fx-$f/suite.log" | tr -s ' ')" "$(tail -1 "$V/ab2/fx-$f/suite.log")" \
    "$(awk '{print $1, $2}' "$V/ab2/fx-$f/ab.log" 2>/dev/null | sort | uniq -c | tr -s ' ' | tr '\n' ';')" >> "$G/fxsel.tsv"
done
cat "$G/fxsel.tsv"
