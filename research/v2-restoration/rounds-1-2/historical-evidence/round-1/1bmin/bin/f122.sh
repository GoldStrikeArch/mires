#!/usr/bin/env bash
# f122.sh <tree> <tag> <src> [modes...]: compiles a fixture-122 variant with <tree>'s compiler, runs it with <tree>'s shim, compares with native.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ae104b5a611c870ef
tree=$1; tag=$2; src=$3; shift 3
modes=${*:-rb def}
n=$(basename $src .rs)
exp=${EXPECTED:-$V/native/$n.O.out}
mkdir -p $V/f122
cd $tree
for m in $modes; do
  case $m in
    rb) a="js-profile=release-balanced" ;;
    rbopt0) a="js-profile=release-balanced js-opt=off" ;;
    rbco) a="js-profile=release-balanced js-cleanup=off" ;;
    jir0) a="js-lane=jir js-opt=off" ;;
    def) a="" ;;
  esac
  js=$V/f122/$tag-$m.js
  if ! JS_EXTRA_ARGS="$a" SKIP_BUILD=1 bash scripts/compile_core.sh $src $js > $js.log 2>&1; then echo "$tag $m COMPILE-FAIL"; tail -5 $js.log; continue; fi
  cat runtime/shim.js $js $W/scripts/entry.js | node - > $js.out 2> $js.err; st=$?
  echo "$tag $m size=$(wc -c < $js | tr -d ' ') exit=$st wrong=$(diff $exp $js.out | grep -c '^<') lines=$(diff $exp $js.out | grep '^[0-9]' | tr '\n' ' ')"
done
