#!/usr/bin/env bash
# native.sh <src.rs> [outdir]: builds a mires-style no_std probe natively with a std prelude,
# at -O0 and -O3 with the pinned toolchain, runs both, and checks they agree.
# Prints the -O3 stdout path. Exit 1 when the two disagree or a build fails.
set -uo pipefail
A=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a6
src=$1
name=$(basename "$src" .rs)
d=${2:-$A/native/$name}
mkdir -p "$d"
python3 -I - "$src" "$d/main.rs" <<'PY'
import sys
src = open(sys.argv[1]).read()
src = src.replace('#![no_std]', '').replace('#![no_main]', '')
shim = ('mod prelude { #![allow(dead_code)] '
        'pub fn print_i32(x:i32){println!("{}",x)} pub fn print_u32(x:u32){println!("{}",x)} '
        'pub fn print_usize(x:usize){println!("{}",x)} pub fn print_i64(x:i64){println!("{}",x)} '
        'pub fn print_u64(x:u64){println!("{}",x as i64)} pub fn print_f64(x:f64){println!("{}",x)} '
        'pub fn print_bool(x:bool){println!("{}",x)} pub fn print_str(s:&str){println!("{}",s)} }')
marker = '#[path = "prelude.rs"]\nmod prelude;'
assert marker in src, "probe must use the shared prelude"
src = src.replace(marker, shim)
src = src.replace('#[unsafe(no_mangle)]\npub fn rust_entry()', 'pub fn rust_entry()')
src += '\nfn main(){ rust_entry(); }\n'
open(sys.argv[2], 'w').write(src)
PY
for opt in 0 3; do
  rustc +nightly-2026-07-27 --edition 2021 -C opt-level=$opt -A warnings -o "$d/main-O$opt" "$d/main.rs" 2> "$d/build-O$opt.log" \
    || { echo "NATIVE BUILD FAIL -O$opt"; head -30 "$d/build-O$opt.log"; exit 1; }
  "$d/main-O$opt" > "$d/stdout-O$opt" 2> "$d/stderr-O$opt"; echo "$?" > "$d/exit-O$opt"
done
if cmp -s "$d/stdout-O0" "$d/stdout-O3" && cmp -s "$d/exit-O0" "$d/exit-O3"; then
  echo "$d/stdout-O3"
else
  echo "NATIVE O0/O3 DISAGREE for $name"; diff "$d/stdout-O0" "$d/stdout-O3" | head; exit 1
fi
