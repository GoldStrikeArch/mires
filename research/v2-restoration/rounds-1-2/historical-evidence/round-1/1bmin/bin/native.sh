#!/usr/bin/env bash
# native.sh <src.rs> <outdir>: transplant to std, build at -O and -O0, run both, report agreement.
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad
src=$1; od=$2; mkdir -p $od
n=$(basename $src .rs)
python3 $V/verify1b-r3/transplant.py $src $od/$n.native.rs
for o in O O0; do
  flag="-O"; [ $o = O0 ] && flag="-Copt-level=0"
  rustc +nightly-2026-07-27 --edition 2021 $flag -Awarnings --crate-name native_fx -o $od/${n}_$o $od/$n.native.rs > $od/$n.$o.err 2>&1 || { echo "NATIVE-COMPILE-FAIL $o"; cat $od/$n.$o.err | head -30; exit 1; }
  $od/${n}_$o > $od/$n.$o.out; echo "exit=$?" >> $od/$n.$o.err
done
if cmp -s $od/$n.O.out $od/$n.O0.out; then echo "O==O0 ($(wc -l < $od/$n.O.out | tr -d ' ') lines)"; else echo "O!=O0"; diff $od/$n.O.out $od/$n.O0.out; fi
