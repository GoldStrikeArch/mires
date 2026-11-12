#!/usr/bin/env bash
# xc.sh <root> <label>: the two-crate probe (src/XC) natively and in four modes with the compiler
# at <root>; prints one line per mode.
set -uo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
root=$1; label=$2
src=$V/src/XC
nat=$V/native/xc
mkdir -p "$nat"
chan=nightly-2026-07-27
python3 -I "$V/tools/nativize.py" "$src/xc_main.rs" "$nat/main.rs"
for opt in 0 3; do
  rustc +$chan --edition 2021 -C opt-level=$opt -A warnings --crate-type rlib --crate-name xc_lib -o "$nat/libxc_lib-O$opt.rlib" "$src/xc_lib.rs" 2> "$nat/lib-O$opt.log" || { echo "native lib fail"; head -20 "$nat/lib-O$opt.log"; exit 1; }
  rustc +$chan --edition 2021 -C opt-level=$opt -A warnings --extern xc_lib="$nat/libxc_lib-O$opt.rlib" -o "$nat/main-O$opt" "$nat/main.rs" 2> "$nat/main-O$opt.log" || { echo "native main fail"; head -20 "$nat/main-O$opt.log"; exit 1; }
  "$nat/main-O$opt" > "$nat/stdout-O$opt"
done
cmp -s "$nat/stdout-O0" "$nat/stdout-O3" || { echo "NATIVE O0/O3 DISAGREE"; exit 1; }
echo "native: $(tr '\n' ' ' < "$nat/stdout-O3")"
for mode in def jiroff rb rbz; do
  out=$V/out/XC/$label/$mode
  mkdir -p "$out"
  (
    unset JSC_RUST_OPT_LEVEL JSC_RUST_MIR_INLINE
    case "$mode" in
      def) export JS_EXTRA_ARGS="" ;;
      jiroff) export JS_EXTRA_ARGS="js-lane=jir js-opt=off" ;;
      rb) export JS_EXTRA_ARGS="js-profile=release-balanced" ;;
      rbz) export JS_EXTRA_ARGS="js-profile=release-balanced"; export JSC_RUST_OPT_LEVEL=z JSC_RUST_MIR_INLINE=on ;;
    esac
    cd "$root" || exit 1
    SKIP_BUILD=1 bash scripts/compile_core.sh "$src/xc_lib.rs" "$out/libxc_lib.rlib" --crate-type rlib --crate-name xc_lib > "$out/lib.log" 2>&1 || { echo "$label $mode LIB COMPILE FAIL: $(grep -m1 '^error' "$out/lib.log" | cut -c1-200)"; exit 0; }
    SKIP_BUILD=1 bash scripts/compile_core.sh "$src/xc_main.rs" "$out/xc_main.js" --extern "xc_lib=$out/libxc_lib.rlib" > "$out/main.log" 2>&1 || { echo "$label $mode MAIN COMPILE FAIL: $(grep -m1 '^error' "$out/main.log" | cut -c1-200)"; exit 0; }
    cat "$root/runtime/shim.js" "$out/xc_main.js" "$root/scripts/entry.js" | node - > "$out/stdout" 2> "$out/stderr"
    code=$?
    if [ $code = 0 ] && cmp -s "$nat/stdout-O3" "$out/stdout"; then echo "$label $mode PASS"; else echo "$label $mode FAIL exit=$code: $(tr '\n' ' ' < "$out/stdout" | cut -c1-200) $(grep -m1 Error "$out/stderr" | cut -c1-150)"; fi
  )
done
