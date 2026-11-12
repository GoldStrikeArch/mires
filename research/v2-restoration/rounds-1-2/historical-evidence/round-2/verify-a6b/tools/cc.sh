#!/usr/bin/env bash
# cc.sh <src.rs> <mode> <keepdir> [extra backend args...]
# Compiles one probe with the worktree's compiler (fix linker) in one mode, and keeps the link
# manifest and objects in <keepdir> so ln.sh can relink it with other link options and linkers.
# Modes: def, jiropt (js-lane=jir js-opt=on), rb (release-balanced), hoist (js-hoist-consts=on),
#        rbhoist, rbnc (rb + js-cleanup=off), rbnomin (rb + js-minify=off),
#        rbz (rb + JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on).
# The direct build lands in <keepdir>/direct/. <keepdir>/args.txt holds the mode's backend args.
set -uo pipefail
source "$(dirname "$0")/env.sh"
src=$1; mode=$2; keep=$3; shift 3
extra="$*"
unset JSC_RUST_OPT_LEVEL JSC_RUST_MIR_INLINE
case "$mode" in
  def) args="" ;;
  jiropt) args="js-lane=jir js-opt=on" ;;
  rb) args="js-profile=release-balanced" ;;
  hoist) args="js-hoist-consts=on" ;;
  rbnc) args="js-profile=release-balanced js-cleanup=off" ;;
  rbnomin) args="js-profile=release-balanced js-minify=off" ;;
  rbhoist) args="js-profile=release-balanced js-hoist-consts=on" ;;
  rbz) args="js-profile=release-balanced"; export JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on ;;
  *) echo "unknown mode $mode"; exit 2 ;;
esac
rm -rf "$keep"; mkdir -p "$keep/direct"
export JS_EXTRA_ARGS="$args${extra:+ $extra}"
export KEEP_DIR=$keep REAL_LINK=$FIX_LINK JSC_LINK_BIN=$V/tools/capture-link.sh
# Probes compile in parallel; nothing builds a sysroot in this worktree while they do.
export RCGJS_INTEGRATION_LOCK_HELD=$W/build/.integration-lock
(cd "$W" && SKIP_BUILD=1 bash scripts/compile_core.sh "$src" "$keep/direct/program.js") > "$keep/compile.log" 2>&1
status=$?
echo "$status" > "$keep/compile.status"
sha=$(shasum -a 256 "$W/target/release/librustc_codegen_mires.dylib" | awk '{print $1}')
printf 'js-producer=%s %s\n' "$sha" "$args" > "$keep/args.txt"
[ -f "$keep/manifest.bin" ] || { echo "cc.sh: no manifest captured for $src ($mode), see $keep/compile.log" >&2; exit 1; }
exit 0
