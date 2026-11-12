#!/usr/bin/env bash
# ln.sh <keepdir> <base|fix|/path/to/jsc-link> <outdir> [link args...]
# Relinks the program cc.sh kept, with the mode's args plus <link args>, into <outdir>/program.js
# (and the chunk files beside it). Writes <outdir>/link.log and <outdir>/status (the linker's exit).
set -uo pipefail
source "$(dirname "$0")/env.sh"
keep=$1; which=$2; out=$3; shift 3
case "$which" in base) link=$BASE_LINK ;; fix) link=$FIX_LINK ;; *) link=$which ;; esac
rm -rf "$out"; mkdir -p "$out"
# shellcheck disable=SC2046
if ! "$RELINK" make "$keep/manifest.bin" "$out/manifest.bin" "$out/program.js" $(cat "$keep/args.txt") "$@" > "$out/link.log" 2>&1; then
  echo opts > "$out/status"; exit 0
fi
"$link" --manifest "$out/manifest.bin" >> "$out/link.log" 2>&1
echo "$?" > "$out/status"
