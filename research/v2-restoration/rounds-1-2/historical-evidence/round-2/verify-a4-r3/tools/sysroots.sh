#!/usr/bin/env bash
# sysroots.sh <root> [modes...]
set -uo pipefail
W=$1; shift
modes=${*:-def jiroff rb rbz}
for mode in $modes; do
  (
    unset JSC_RUST_OPT_LEVEL JSC_RUST_MIR_INLINE
    case "$mode" in
      def) export JS_EXTRA_ARGS="" ;;
      jiroff) export JS_EXTRA_ARGS="js-lane=jir js-opt=off" ;;
      rb) export JS_EXTRA_ARGS="js-profile=release-balanced" ;;
      rbz) export JS_EXTRA_ARGS="js-profile=release-balanced"; export JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on ;;
    esac
    cd "$W" && SKIP_BUILD=1 bash scripts/build_sysroot.sh > /dev/null 2>&1 && echo "sysroot $mode ok" || echo "sysroot $mode FAILED"
  )
done
