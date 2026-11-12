#!/usr/bin/env bash
# run.sh: compiles the repro with the base and the change linker through scripts/compile_core.sh of
# the probe copy (byte copies of the implementer worktree's scripts, backend dylib and sysroots).
S=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/verify4b
P=$S/probe/src
cd $S/repro
for name in inline_lost_write inline_lost_write_core; do
  for args in "js-profile=release-balanced" "js-lane=jir js-opt=on"; do
    for l in base change; do
      out=$S/repro/out/$name.$l.$(echo "$args" | tr ' =' '__').js
      JS_EXTRA_ARGS="$args" JSC_LINK_BIN=$S/bin/jsc-link-$l RCGJS_WITNESS_CHECK=1 bash $P/scripts/compile_core.sh $S/repro/$name.rs $out > /dev/null 2> $out.compile.log
      got=$(cat $P/runtime/shim.js $out $P/scripts/entry.js | node - 2>/dev/null)
      if [ "$got" = "$(cat $name.expected)" ]; then verdict=RIGHT; else verdict=WRONG; fi
      echo "$name [$args] linker=$l: last line $(echo "$got" | tail -1) (native $(tail -1 $name.expected)) -> $verdict"
    done
  done
done
