#!/usr/bin/env bash
# native.sh <name> [rounds]: builds <name>_body.rs natively at -O0 and -O3 and prints both outputs.
set -u
P=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min/probe
n=$1
rounds=${2:-1000}
cd "$P" || exit 1
mkdir -p nat
{
	echo 'extern crate alloc;'
	echo "#[allow(dead_code)] const ROUNDS: u32 = $rounds;"
	echo '#[allow(dead_code)] fn print_bool(x: bool) { println!("{}", x) }'
	echo '#[allow(dead_code)] fn print_usize(x: usize) { println!("{}", x) }'
	echo '#[allow(dead_code)] fn print_u32(x: u32) { println!("{}", x) }'
	echo '#[allow(dead_code)] fn print_str(x: &str) { println!("{}", x) }'
	echo "include!(\"${n}_body.rs\");"
	echo 'fn main() { rust_entry() }'
} >"${n}_native.rs"
for o in 0 3; do
	rustc --edition 2021 -C opt-level=$o -A dead_code -A unused "${n}_native.rs" -o "nat/${n}_$o" || exit 1
	"./nat/${n}_$o" >"nat/${n}_$o.out"
done
if cmp -s "nat/${n}_0.out" "nat/${n}_3.out"; then echo "$n: O0 == O3"; else echo "$n: O0 != O3"; fi
tr '\n' ' ' <"nat/${n}_3.out"
echo
