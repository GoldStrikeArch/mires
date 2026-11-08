#!/usr/bin/env bash
# Exercise the real suite driver with cheap compiler stubs. Compiler semantics are
# covered by the real fixture suite; these checks prevent missing suites passing.
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
root="$work/root"
mkdir -p "$root/scripts" "$root/examples/tests" "$root/examples/core-tests" \
	"$root/runtime" "$root/mires-js/src" "$root/target/release" "$work/bin"
# An optional retained driver supports checking that the regression goes red.
cp "${1:-$script_dir/test.sh}" "$root/scripts/test.sh"
cp "$script_dir/sysroot-cache.sh" "$script_dir/integration-lock.sh" "$root/scripts/"
printf 'channel = "nightly-test"\n' >"$root/rust-toolchain.toml"
touch "$root/target/release/librustc_codegen_mires.dylib" "$root/target/release/librustc_codegen_mires.so"
touch "$root/examples/tests/smoke.rs" "$root/examples/core-tests/core_smoke.rs" "$root/runtime/shim.js"
printf '42\n' >"$root/examples/tests/smoke.expected"
printf '42\n' >"$root/examples/core-tests/core_smoke.expected"
printf 'console.log(rust_entry());\n' >"$root/scripts/entry.js"
cat >"$root/scripts/compile.sh" <<'EOF'
#!/usr/bin/env bash
printf 'function rust_entry(){return 42;}\n' >"$2"
EOF
cp "$root/scripts/compile.sh" "$root/scripts/compile_core.sh"
cat >"$root/scripts/build_sysroot.sh" <<'EOF'
#!/usr/bin/env bash
touch "$(dirname "$0")/unexpected-build"
exit 1
EOF
cat >"$work/bin/rustc" <<'EOF'
#!/usr/bin/env bash
printf 'rustc 1.99.0\ncommit-hash: harness\nhost: test-host\n'
EOF
cat >"$work/bin/cargo" <<'EOF'
#!/usr/bin/env bash
mkdir -p target/debug
touch target/debug/libjs_extern_macro.dylib target/debug/libjs_extern_macro.so
EOF
chmod +x "$root/scripts/compile.sh" "$root/scripts/compile_core.sh" "$root/scripts/build_sysroot.sh" "$work/bin/"*
export PATH="$work/bin:$PATH" SKIP_BUILD=1
unset JSC_SYSROOT_ARGS JSC_RUST_OPT_LEVEL JSC_TARGET REQUIRE_CORE RCGJS_INTEGRATION_LOCK_HELD
export JS_EXTRA_ARGS='js-profile=release-size js-lane=jir'
selected=$(bash "$root/scripts/sysroot-cache.sh" path)
mkdir -p "$selected/lib/rustlib/wasm32-unknown-unknown/lib"
bash "$root/scripts/test.sh" >"$work/keyed.log" 2>&1
if ! grep -q '^2/2 passed$' "$work/keyed.log"; then
	cat "$work/keyed.log" >&2
	echo 'keyed sysroot fixture suite did not run 2/2 tests' >&2
	exit 1
fi
! grep -q 'core suite skipped' "$work/keyed.log"
[ ! -e "$root/build/sysroot" ]
[ ! -e "$root/scripts/unexpected-build" ]
mv "$selected" "$selected.saved"
if bash "$root/scripts/test.sh" >"$work/missing.log" 2>&1; then
	echo 'missing core suite incorrectly passed' >&2
	exit 1
fi
grep -q 'the real-core suite is required' "$work/missing.log"
REQUIRE_CORE=0 bash "$root/scripts/test.sh" >"$work/partial.log" 2>&1
grep -q '^1/1 passed$' "$work/partial.log"
grep -q 'core suite skipped' "$work/partial.log"
if REQUIRE_CORE=0 bash "$root/scripts/test.sh" core_smoke >"$work/named.log" 2>&1; then
	echo 'unexecuted named core test incorrectly passed' >&2
	exit 1
fi
grep -q 'the real-core suite is required' "$work/named.log"
! grep -q '0/0 passed' "$work/named.log"
mv "$selected.saved" "$selected"
# An explicit selector has the same authority here as in compile_core.sh.
JSC_SYSROOT_ARGS='js-profile=release-size' JS_EXTRA_ARGS='js-opt=off' \
	bash "$root/scripts/test.sh" core_smoke >"$work/override.log" 2>&1
grep -q '^1/1 passed$' "$work/override.log"
mv "$root/examples/core-tests/core_smoke.rs" "$root/examples/core-tests/core_smoke.saved"
if bash "$root/scripts/test.sh" >"$work/corpus.log" 2>&1; then
	echo 'missing core corpus incorrectly passed' >&2
	exit 1
fi
grep -q 'no core tests found' "$work/corpus.log"
bash "$root/scripts/test.sh" smoke >"$work/freestanding.log" 2>&1
grep -q '^1/1 passed$' "$work/freestanding.log"
printf 'fixture harness checks passed (keyed, missing, partial, named, override, corpus, freestanding)\n'
