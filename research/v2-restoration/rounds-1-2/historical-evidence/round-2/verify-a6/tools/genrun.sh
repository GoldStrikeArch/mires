#!/usr/bin/env bash
# genrun.sh <tag> <prefix> <n>...: generated programs <prefix>_<n>, unsplit in four modes and split
# into three chunks (requested in two orders) in the default profile and release-balanced.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
tag=$1; prefix=$2; shift 2
for n in "$@"; do
  bash "$A/tools/run.sh" "$tag" "${prefix}_$n"
  CMODES="def rb" bash "$A/tools/chunkrun.sh" "$tag" "${prefix}_$n" a:chunk_a,b:chunk_b,c:chunk_c a:chunk_a,b:chunk_b,c:chunk_c c:chunk_c,a:chunk_a,b:chunk_b
done
