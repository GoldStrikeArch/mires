#!/usr/bin/env bash
# hand.sh <probe name> <mode> <calls csv | -> <split>...
# One handwritten probe (src/<name>.rs), compiled once in <mode>, then relinked by both linkers
# for each <split> (a comma separated list of chunk:entry pairs and link options) and loaded in
# every file order; <calls> are the function entries to call, compared with the native run.
# Rows go to results/hand-<name>.<mode>.tsv and are printed.
set -uo pipefail
source "$(dirname "$0")/env.sh"
name=$1; mode=$2; calls=$3; shift 3
src=$V/src/$name.rs
keep=$V/keep/$name.$mode
nat=-
if [ "$calls" != - ]; then
  nat=$(bash "$A6/tools/native.sh" "$src" "$V/native/$name") || { echo "$name NATIVE-FAIL: $nat"; exit 1; }
fi
first=${calls%%,*}
[ "$calls" = - ] && first=${1%%,*}
bash "$V/tools/cc.sh" "$src" "$mode" "$keep" js-modules=esm "js-chunk=$first" || { tail -5 "$keep/compile.log"; exit 1; }
[ "$(cat "$keep/compile.status")" = 0 ] || { echo "$name $mode: the capture compile failed"; grep -m3 '^error' "$keep/compile.log"; }
jobs=$V/jobs/hand-$name.$mode.jobs
: > "$jobs"
n=0
for split in "$@"; do
  n=$((n + 1))
  args="js-modules=esm"
  for word in $(printf '%s' "$split" | tr ',' ' '); do
    case "$word" in js-*) args="$args $word" ;; *) args="$args js-chunk=$word" ;; esac
  done
  echo "$name.$mode.$n[$split]|$keep|$V/out/hand/$name.$mode.$n|$nat|$calls|$args" >> "$jobs"
done
python3 "$V/tools/probe.py" "$jobs" "$V/results/hand-$name.$mode.tsv" 4
cat "$V/results/hand-$name.$mode.tsv"
