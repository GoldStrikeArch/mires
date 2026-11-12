#!/usr/bin/env bash
# A probe root for the fix compiler with its own integration lock: everything is a symlink into the
# verifier worktree except build/, whose sysroots directory is the worktree's (keyed by dylib hash).
set -euo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-a3724c72be1271b05
B=$V/fixroot
mkdir -p "$B/build"
for e in backend Cargo.toml Cargo.lock examples jsc-link patches rcgjs-object runtime rust-toolchain.toml scripts experiments mires-js view-abi view-async js-macro js-extern-macro structurizer rustc-cleanup contract target; do
  [ -e "$W/$e" ] && ln -sfn "$W/$e" "$B/$e"
done
ln -sfn "$W/build/sysroots" "$B/build/sysroots"
ls -la "$B" "$B/build"
