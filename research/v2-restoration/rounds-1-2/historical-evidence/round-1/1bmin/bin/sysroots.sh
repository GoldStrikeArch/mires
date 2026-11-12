#!/usr/bin/env bash
# sysroots.sh <tree> <tag>: builds the sysroot for each of the five modes in <tree> (SKIP_BUILD=1), sequentially.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
tree=$1; tag=$2
cd $tree || exit 1
i=0
for m in "js-profile=release-balanced" "js-profile=release-balanced js-opt=off" "js-profile=release-balanced js-cleanup=off" "js-lane=jir js-opt=off" ""; do
  i=$((i+1))
  start=$(date +%s)
  JS_EXTRA_ARGS="$m" SKIP_BUILD=1 bash scripts/build_sysroot.sh > $V/logs/sysroot-$tag-$i.log 2>&1
  echo "sysroot $tag [$m] exit=$? $(( $(date +%s) - start ))s"
done
echo SYSROOTS-DONE $tag
