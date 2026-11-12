#!/usr/bin/env bash
# ab.sh <root> <tag> <JS_EXTRA_ARGS> : scripts/test.sh in <root> once with main's linker and once
# with the fix's (the root's own), outputs copied to ab/<tag>/{base,fix}. Same dylib and sysroot.
# Extra env (JSC_RUST_OPT_LEVEL, JSC_RUST_MIR_INLINE) passes through.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
root=$1; tag=$2; args=$3
cd "$root" || exit 2
for linker in base fix; do
  o=$A/ab/$tag/$linker
  rm -rf "$o"; mkdir -p "$o/core"
  rm -f build/*.js build/core/*.js
  if [ "$linker" = base ]; then export JSC_LINK_BIN=$A/main-src/target/release/jsc-link; else unset JSC_LINK_BIN; fi
  SKIP_BUILD=1 JS_EXTRA_ARGS="$args" bash scripts/test.sh --require-core > "$o/test.log" 2>&1
  echo "exit $?" >> "$o/test.log"
  cp build/*.js "$o/" 2>/dev/null; cp build/core/*.js "$o/core/" 2>/dev/null
  echo "$tag $linker done: $(tail -1 "$o/test.log") files=$(ls "$o/core" | wc -l | tr -d ' ')"
done
