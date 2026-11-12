#!/usr/bin/env bash
# JSC_LINK_BIN wrapper: every link a suite performs is done twice, once by the cf80163 linker on a
# side copy (tools/relink `side`: copied inputs, output in a side directory) and once, for real,
# by the linker under test. The two results are compared byte for byte and one line is appended to
# $AB_DIR/ab.log:
#   SAME|DIFF|STATUS  <kind>  <real output>  [what differs]
# The suite sees only the linker under test: its exit status, stdout and stderr.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6b
BASE_LINK=$V/base-src/target/release/jsc-link
FIX_LINK=${AB_FIX_LINK:-/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-acddce572a795adec/target/release/jsc-link}
RELINK=$V/tools/relink/target/release/relink
AB_DIR=${AB_DIR:-$V/ab2/misc}
if [ "$1" != --manifest ]; then exec "$FIX_LINK" "$@"; fi
mkdir -p "$AB_DIR"
side=$(mktemp -d "$AB_DIR/link.XXXXXX")
if ! "$RELINK" side "$2" "$side" 2> "$side/relink.err"; then
  echo "TOOL  -  -  relink side failed, see $side/relink.err" >> "$AB_DIR/ab.log"
  exec "$FIX_LINK" "$@"
fi
kind=$(head -1 "$side/info.txt")
"$BASE_LINK" --manifest "$side/manifest.bin" > "$side/base.stdout" 2> "$side/base.stderr"
base=$?
case "$kind" in
stdout)
  "$FIX_LINK" "$@" 2> "$side/fix.stderr" | tee "$side/fix.stdout"
  fix=${PIPESTATUS[0]}
  ;;
*)
  "$FIX_LINK" "$@" 2> "$side/fix.stderr"
  fix=$?
  ;;
esac
cat "$side/fix.stderr" >&2
what=""
if [ "$base" -ne "$fix" ]; then
  verdict=STATUS; what="base exit $base, fix exit $fix"
else
  verdict=SAME
  case "$kind" in
  program*)
    real=${kind#program }
    for f in "$side/out"/*; do
      [ -e "$f" ] || continue
      if ! cmp -s "$f" "$(dirname "$real")/$(basename "$f")"; then verdict=DIFF; what="$what $(basename "$f")"; fi
    done
    [ "$fix" -eq 0 ] && [ ! -e "$side/out/$(basename "$real")" ] && { verdict=DIFF; what="$what base-wrote-no-output"; }
    ;;
  stdout)
    cmp -s "$side/base.stdout" "$side/fix.stdout" || { verdict=DIFF; what="stdout"; }
    ;;
  prune)
    while IFS=$'\t' read -r realpath sidepath; do
      [ -n "$sidepath" ] || continue
      cmp -s "$realpath" "$sidepath" || { verdict=DIFF; what="$what $(basename "$realpath")"; }
    done < <(tail -n +2 "$side/info.txt")
    ;;
  esac
  if [ "$fix" -ne 0 ]; then
    cmp -s "$side/base.stderr" "$side/fix.stderr" && what="$what both-refused-same-message" || what="$what both-refused-messages-differ"
  fi
fi
echo "$verdict  $kind  $what" >> "$AB_DIR/ab.log"
# Keep the side copy only when there is something to look at.
if [ "$verdict" = SAME ] && [ "$fix" -eq 0 ]; then rm -rf "$side"; fi
exit "$fix"
