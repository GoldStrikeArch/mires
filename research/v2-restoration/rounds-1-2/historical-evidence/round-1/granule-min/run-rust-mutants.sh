#!/usr/bin/env bash
# run-rust-mutants.sh: each linker mutant (a) compiles fixtures 122-124 under release-balanced and
# runs them as test.sh does, against their .expected, and (b) is applied to the source for the
# committed Rust unit tests (`cargo test -p jsc-link`), then reverted.
set -u
G=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992
cd "$W" || exit 1
out=$G/rmut
mkdir -p "$out"
for m in r1-call-arg-at-drop r2-drop-cut-not-released m2c-from-utf8-unchecked-releases m2d-str-from-raw-parts-releases r3-adapter-call-releases; do
	echo "== $m $(date +%T)"
	if [ -x "$G/mt/jsc-link.$m" ]; then
		for f in 122_consumed_block_addresses 123_moved_block_addresses 124_read_block_addresses; do
			JSC_LINK_BIN=$G/mt/jsc-link.$m JS_EXTRA_ARGS="js-profile=release-balanced" \
				bash scripts/compile_core.sh "examples/core-tests/$f.rs" "$out/$m.$f.js" >"$out/$m.$f.log" 2>&1
			if cat runtime/shim.js "$out/$m.$f.js" scripts/entry.js | node - >"$out/$m.$f.out" 2>&1 &&
				cmp -s "$out/$m.$f.out" "examples/core-tests/$f.expected"; then
				echo "  fixture $f: pass"
			else
				echo "  fixture $f: FAIL ($(tr '\n' ' ' <"$out/$m.$f.out" | cut -c1-110))"
			fi
		done
	fi
	python3 "$G/rust_mutant.py" apply "$m" >/dev/null || exit 1
	cargo test -p jsc-link --lib -- adapters only_the_cut_in_front_of_a_drop >"$out/$m.cargo.log" 2>&1
	python3 "$G/rust_mutant.py" revert "$m" >/dev/null
	grep -E "^test .* FAILED$" "$out/$m.cargo.log" | sed 's/^/  cargo: /'
	grep -E "^test result" "$out/$m.cargo.log" | head -1 | sed 's/^/  cargo: /'
done
git -C "$W" diff --stat -- jsc-link/src/plan/representation.rs jsc-link/src/lower/js/ptr.rs jsc-link/src/lower/js/adapters.rs
echo "== done $(date +%T)"
