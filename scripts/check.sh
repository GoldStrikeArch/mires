#!/usr/bin/env bash
#
# The cheap pre-landing check every workstream runs before it reports done:
# formatting, clippy with warnings denied, and the workspace unit tests. The
# fixture suites (test.sh, emit-test.sh, ...) are separate because they need
# sysroots; this one needs only cargo.
#
#   scripts/check.sh

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$script_dir/.." && pwd)
# shellcheck source=integration-lock.sh
source "$script_dir/integration-lock.sh"
cd "$root"

# The property tests take their corpus from `build/`. Seed a clean checkout with three small,
# complementary real-core fixtures before running them; otherwise PEA, range, DCE, and summary
# tests can all report success after examining zero functions.
if ! find build/corpus -type f -name '*.o' -print -quit 2>/dev/null | grep -q .; then
	scripts/build.sh >/dev/null
	JS_EXTRA_ARGS='js-lane=jir js-opt=on' SKIP_BUILD=1 scripts/build_sysroot.sh >/dev/null
	for fixture in 60_virtual_struct_merge 77_pea_merge_materialize 81_bounds_range; do
		JS_EXTRA_ARGS='js-lane=jir js-opt=on' \
			scripts/compile_core.sh "examples/core-tests/$fixture.rs" \
			"build/corpus/$fixture.js" >/dev/null
	done
fi
export RCGJS_SSA_FIXTURES_REQUIRED=1
export RCGJS_WITNESS_CHECK=1

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# The UI integration is not part of the standalone compiler workspace.
node --test scripts/native-values.test.mjs scripts/host-future-driver.test.mjs \
    scripts/minimal-shim.test.mjs scripts/runtime-*.test.mjs
bash scripts/sysroot-cache.test.sh
# Removing no-op landing pads must not change which bodies keep executable cleanup.
bash scripts/noop-landing-pads-test.sh
