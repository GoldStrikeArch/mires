#!/usr/bin/env bash
# transplant.sh <fixture.rs> <name>: the fixture moved onto std (the prelude's printers become
# println!), built at -O0 and -O3; prints both outputs' agreement and the -O3 output.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
src=$1
name=$2
out=$G/probe/nat
mkdir -p "$out"
grep -v -e '^#!\[no_std\]' -e '^#!\[no_main\]' -e '^#\[path = "prelude.rs"\]' -e '^mod prelude;' \
	-e '^use prelude::\*;' "$src" >"$out/${name}_native.rs"
cat >>"$out/${name}_native.rs" <<'EOF'
#[allow(dead_code)] fn print_bool(x: bool) { println!("{}", x) }
#[allow(dead_code)] fn print_u32(x: u32) { println!("{}", x) }
#[allow(dead_code)] fn print_usize(x: usize) { println!("{}", x) }
#[allow(dead_code)] fn print_str(x: &str) { println!("{}", x) }
fn main() { rust_entry() }
EOF
for o in 0 3; do
	rustc --edition 2021 -C opt-level=$o -A unused "$out/${name}_native.rs" -o "$out/$name.$o" || exit 1
	"$out/$name.$o" >"$out/$name.$o.out"
done
if cmp -s "$out/$name.0.out" "$out/$name.3.out"; then echo "$name: O0 == O3"; else echo "$name: O0 != O3"; fi
cat "$out/$name.3.out"
