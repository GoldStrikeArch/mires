#!/usr/bin/env bash
# run.sh <tag> <src.rs>... : native O0/O3, then the four modes on fix (and main when COMPILERS says
# so), then the table and, with SHOW=1, each program's native / fix / main stdout on one line.
set -uo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
tag=$1; shift
rm -rf "$V/out/$tag"
bash "$V/tools/natall.sh" "$@" 2>&1 | grep -v '^native-done' | head -40
bash "$V/tools/par.sh" "$tag" "$@" > "$V/logs/par-$tag.log" 2>&1
python3 "$V/tools/table.py" "$tag" ${MSG:+--msg}
if [ "${SHOW:-0}" = 1 ]; then
  for f in "$@"; do
    n=$(basename "$f" .rs)
    echo "== $n native: $(tr '\n' ' ' < "$V/native/$n/stdout-O3" | cut -c1-${WIDTH:-300})"
    for c in ${COMPILERS:-fix}; do
      for m in ${SHOWMODES:-def rb}; do
        echo "   $c $m: $(tr '\n' ' ' < "$V/out/$tag/$c/$n.$m.stdout" | cut -c1-${WIDTH:-300})"
      done
    done
  done
fi
