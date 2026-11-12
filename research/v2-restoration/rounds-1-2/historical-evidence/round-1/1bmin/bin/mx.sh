#!/usr/bin/env bash
# mx.sh <tree> <arm> <mode-tag> <set> [names...]
#   Compiles and runs fixtures of corpus/<set> with <tree>'s compiler and runtime in one mode.
#   mode-tag: rb | rbopt0 | rbco | jir0 | def
#   Output: out/<arm>/<mode-tag>/<set>-<name>.js{,.out,.err,.log,.rc}
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
tree=$1; arm=$2; mode=$3; set=$4; shift 4
case $mode in
  rb) args="js-profile=release-balanced" ;;
  rbopt0) args="js-profile=release-balanced js-opt=off" ;;
  rbco) args="js-profile=release-balanced js-cleanup=off" ;;
  jir0) args="js-lane=jir js-opt=off" ;;
  def) args="" ;;
  *) echo "unknown mode $mode"; exit 2 ;;
esac
fxdir=$V/corpus/$set
od=$V/out/$arm/$mode; mkdir -p $od
cd $tree || exit 1
export SKIP_BUILD=1 JS_EXTRA_ARGS="$args"
if [ $# -eq 0 ]; then set -- $(ls $fxdir/*.rs | xargs -n1 basename | sed 's/\.rs$//' | grep -v '^prelude$'); fi
for n in "$@"; do
  f=$fxdir/$n.rs
  js=$od/$set-$n.js
  rm -f $js $js.out $js.err $js.rc
  if ! bash scripts/compile_core.sh $f $js > $js.log 2>&1; then echo "COMPILE-FAILED" > $js.rc; echo "COMPILE-FAIL $arm $mode $set/$n"; continue; fi
  if grep -q 'fn host_throw' $f; then node $V/../verify1b-r3/cleanup_check.mjs $js runtime/shim.js > $js.out 2> $js.err; st=$?
  else cat runtime/shim.js $js scripts/entry.js | node - > $js.out 2> $js.err; st=$?; fi
  echo $st > $js.rc
  if [ $st = 0 ] && cmp -s $js.out $fxdir/$n.expected; then echo "PASS $arm $mode $set/$n"
  else echo "FAIL $arm $mode $set/$n exit=$st difflines=$(diff $fxdir/$n.expected $js.out | grep -c '^[<>]')"; fi
done
