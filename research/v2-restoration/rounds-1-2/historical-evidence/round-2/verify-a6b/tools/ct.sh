#!/usr/bin/env bash
# ct.sh: what scripts/chunk-test.sh reports for `.chunks` files that must NOT pass, and for a
# linker that does nothing. Runs in the scratch copy ct-src (e7880e0's tree with the worktree's
# dylib, jsc-link and sysroots), on a temporary fixture 901_ct that is a copy of fixture 266.
# One line per case: the case, the suite's exit status, and its PASS/FAIL lines.
set -uo pipefail
source "$(dirname "$0")/env.sh"
R=$V/ct-src
core=$R/examples/core-tests
cp "$core/266_chunk_static_entry_refusals.rs" "$core/901_ct.rs"
cp "$core/266_chunk_static_entry_refusals.expected" "$core/901_ct.expected"
run() { # run <case name> <expect: fail|pass> [env assignment]
  local name=$1 expect=$2 extra=${3:-}
  local log=$V/logs/ct-$name.log
  (cd "$R" && env $extra SKIP_BUILD=1 JS_EXTRA_ARGS='js-lane=jir js-opt=on' bash scripts/chunk-test.sh 901_ct) > "$log" 2>&1
  local status=$?
  local got=pass; [ "$status" -ne 0 ] && got=fail
  local mark=as-wanted; [ "$got" != "$expect" ] && mark="NOT-AS-WANTED(wanted $expect)"
  echo "$name: exit $status [$mark] $(grep -E '^(PASS|FAIL)' "$log" | sed 's/901_ct//' | tr '\n' ';' | cut -c1-330)"
}
chunks=$core/901_ct.chunks
printf 'a:chunk_a b:chunk_b\nrefuse: s:ENTRY_S a:chunk_a b:chunk_b -- import each other, and module level initializers read bindings\n' > "$chunks"; run control pass
printf 'refuse: a:chunk_a b:chunk_b -- import each other\n' > "$chunks"; run refuse-but-compiles fail
printf 'refuse: s:ENTRY_S a:chunk_a b:chunk_b -- some text the linker does not say\n' > "$chunks"; run refuse-wrong-text fail
printf 'refuse: s:ENTRY_S a:chunk_a b:chunk_b -- \n' > "$chunks"; run refuse-empty-text fail
printf 'refuse: s:ENTRY_S a:chunk_a b:chunk_b\n' > "$chunks"; run refuse-no-separator fail
printf 'refuse: x:nope a:chunk_a -- import each other\n' > "$chunks"; run refuse-other-error fail
printf 'refuse: x:nope a:chunk_a -- names no item in this program\n' > "$chunks"; run refuse-other-error-its-own-text pass
printf '  refuse: s:ENTRY_S a:chunk_a b:chunk_b -- import each other\n' > "$chunks"; run refuse-indented fail
printf 'run: a:chunk_a b:chunk_b\n' > "$chunks"; run run-line-only fail
printf '# nothing\n\n' > "$chunks"; run comments-only fail
printf 'run: a:chunk_a z:chunk_z\na:chunk_a b:chunk_b\n' > "$chunks"; run run-names-absent-chunk fail
printf 's:ENTRY_S a:chunk_a\n' > "$chunks"; run static-entry-called-without-run-line fail
printf 'run: a:chunk_a\ns:ENTRY_S a:chunk_a\n' > "$chunks"; run static-entry-with-run-line-but-one-call fail
printf 'a:chunk_a b:chunk_b\nrefuse: s:ENTRY_S a:chunk_a b:chunk_b -- import each other\nrefuse: c:ENTRY_C a:chunk_a b:chunk_b -- refer to each other\n' > "$chunks"
run linker-does-nothing-exit-0 fail JSC_LINK_BIN=/usr/bin/true
run linker-does-nothing-exit-1 fail JSC_LINK_BIN=/usr/bin/false
run linker-is-cf80163 fail "JSC_LINK_BIN=$BASE_LINK"
rm -f "$core"/901_ct.*
