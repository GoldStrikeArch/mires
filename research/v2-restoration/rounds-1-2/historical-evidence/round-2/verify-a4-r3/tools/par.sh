#!/usr/bin/env bash
# par.sh <tag> <src.rs>... : run matrix.sh over the files split across fixroot, fixroot2, fixroot3
# env COMPILERS / MODES pass through to matrix.sh
set -uo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
tag=$1; shift
a=(); b=(); c=(); i=0
for f in "$@"; do
  f=$(cd "$(dirname "$f")" && pwd)/$(basename "$f")
  case $((i % 3)) in 0) a+=("$f");; 1) b+=("$f");; 2) c+=("$f");; esac
  i=$((i+1))
done
[ ${#a[@]} -gt 0 ] && MAINROOT=$V/mainroot ROOT=$V/fixroot bash $V/tools/matrix.sh "$tag" "${a[@]}" > /dev/null &
[ ${#b[@]} -gt 0 ] && MAINROOT=$V/mainroot2 ROOT=$V/fixroot2 bash $V/tools/matrix.sh "$tag" "${b[@]}" > /dev/null &
[ ${#c[@]} -gt 0 ] && MAINROOT=$V/mainroot3 ROOT=$V/fixroot3 bash $V/tools/matrix.sh "$tag" "${c[@]}" > /dev/null &
wait
echo "PASS: $(grep -c ' PASS$' $V/out/$tag/summary.txt)  NOT-PASS: $(grep -vc ' PASS$' $V/out/$tag/summary.txt)"
grep -v " PASS$" $V/out/$tag/summary.txt | sort | head -60
