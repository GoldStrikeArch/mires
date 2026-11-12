#!/usr/bin/env bash
# suites.sh <tree> <arm>: scripts/test.sh --require-core under release-balanced and under the
# default profile, sequentially, into logs/suite-<arm>-{rb,def}.log.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/1bmin
tree=$1; arm=$2
cd $tree
start=$(date +%s)
JS_EXTRA_ARGS="js-profile=release-balanced" SKIP_BUILD=1 bash scripts/test.sh --require-core > $V/logs/suite-$arm-rb.log 2>&1
echo "suite $arm rb exit=$? $(( $(date +%s) - start ))s"
start=$(date +%s)
SKIP_BUILD=1 bash scripts/test.sh --require-core > $V/logs/suite-$arm-def.log 2>&1
echo "suite $arm def exit=$? $(( $(date +%s) - start ))s"
