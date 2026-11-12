#!/usr/bin/env bash
# abfx.sh <tag> <JS_EXTRA_ARGS>: scripts/test.sh --require-core in the worktree, once with the
# cf80163 linker (JSC_LINK_BIN) and once with the linker under test, same dylib and sysroot.
# Every program each run leaves in build/ and build/core/ is copied to ab/<tag>/{base,fix}, with
# the suite's table in test.log. Then the two sets are compared byte for byte.
set -uo pipefail
source "$(dirname "$0")/env.sh"
tag=$1; args=$2
cd "$W" || exit 2
for linker in base fix; do
  o=$V/ab/$tag/$linker
  rm -rf "$o"; mkdir -p "$o/core"
  rm -f build/*.js build/core/*.js
  if [ "$linker" = base ]; then export JSC_LINK_BIN=$BASE_LINK; else unset JSC_LINK_BIN; fi
  SKIP_BUILD=1 JS_EXTRA_ARGS="$args" bash scripts/test.sh --require-core > "$o/test.log" 2>&1
  echo "exit $?" >> "$o/test.log"
  cp build/*.js "$o/" 2>/dev/null; cp build/core/*.js "$o/core/" 2>/dev/null
  echo "$tag $linker: $(tail -1 "$o/test.log") files=$(ls "$o" "$o/core" | grep -c '\.js$')"
done
unset JSC_LINK_BIN
diff -rq "$V/ab/$tag/base" "$V/ab/$tag/fix" > "$V/ab/$tag/diff.txt" 2>&1
echo "$tag differing files: $(grep -c . "$V/ab/$tag/diff.txt")"
