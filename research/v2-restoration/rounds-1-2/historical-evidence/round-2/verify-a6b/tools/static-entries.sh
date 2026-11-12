#!/usr/bin/env bash
# static-entries.sh <mode>...: the reported static-entry programs and fixtures 265/266, split with
# their `static` entry points in several request orders, both linkers, every load order.
set -uo pipefail
source "$(dirname "$0")/env.sh"
core=$W/examples/core-tests
cp "$core/265_chunk_static_entries.rs" "$V/src/fx265.rs"
cp "$core/266_chunk_static_entry_refusals.rs" "$V/src/fx266.rs"
for mode in "$@"; do
  bash "$V/tools/hand.sh" ck_static_entry "$mode" a:chunk_a,b:chunk_b \
    s:ENTRY_S,a:chunk_a,b:chunk_b a:chunk_a,b:chunk_b,s:ENTRY_S b:chunk_b,s:ENTRY_S,a:chunk_a \
    s:ENTRY_S,a:chunk_a s:ENTRY_S,b:chunk_b a:chunk_a,b:chunk_b \
    s:ENTRY_S,a:chunk_a,b:chunk_b,js-chunk-registry=on s:ENTRY_S,a:chunk_a,b:chunk_b,js-chunk-shared=common
  bash "$V/tools/hand.sh" ck_static_entry_oneway "$mode" a:chunk_a,b:chunk_b \
    u:ENTRY_UP,d:ENTRY_DOWN,a:chunk_a,b:chunk_b a:chunk_a,d:ENTRY_DOWN,b:chunk_b,u:ENTRY_UP \
    b:chunk_b,a:chunk_a,u:ENTRY_UP,d:ENTRY_DOWN d:ENTRY_DOWN,u:ENTRY_UP,b:chunk_b,a:chunk_a \
    u:ENTRY_UP,a:chunk_a,b:chunk_b d:ENTRY_DOWN,a:chunk_a,b:chunk_b a:chunk_a,b:chunk_b \
    u:ENTRY_UP,d:ENTRY_DOWN,a:chunk_a,b:chunk_b,js-chunk-registry=on,js-chunk-role=u:island,js-chunk-role=d:island
  bash "$V/tools/hand.sh" ck_static_entry_cycle "$mode" a:chunk_a,b:chunk_b \
    c:ENTRY_C,a:chunk_a,b:chunk_b a:chunk_a,b:chunk_b,c:ENTRY_C a:chunk_a,b:chunk_b
  bash "$V/tools/hand.sh" fx265 "$mode" a:chunk_a,b:chunk_b \
    u:ENTRY_UP,d:ENTRY_DOWN,a:chunk_a,b:chunk_b a:chunk_a,d:ENTRY_DOWN,b:chunk_b,u:ENTRY_UP \
    b:chunk_b,u:ENTRY_UP,a:chunk_a,d:ENTRY_DOWN d:ENTRY_DOWN,b:chunk_b,u:ENTRY_UP,a:chunk_a
  bash "$V/tools/hand.sh" fx266 "$mode" a:chunk_a,b:chunk_b \
    a:chunk_a,b:chunk_b b:chunk_b,a:chunk_a s:ENTRY_S,a:chunk_a,b:chunk_b a:chunk_a,b:chunk_b,s:ENTRY_S \
    c:ENTRY_C,a:chunk_a,b:chunk_b a:chunk_a,c:ENTRY_C,b:chunk_b s:ENTRY_S,c:ENTRY_C,a:chunk_a,b:chunk_b
done
