#!/usr/bin/env bash
# mkroot.sh <rootname> : a probe root for the frozen fix compiler (fix-bin) with its own lock;
# sources are symlinks into the verifier worktree, sysroots shared in fix-sysroots.
set -euo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
W=/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-a3724c72be1271b05
B=$V/$1
mkdir -p "$B/build"
for e in backend Cargo.toml Cargo.lock examples jsc-link patches rcgjs-object runtime rust-toolchain.toml scripts experiments mires-js view-abi view-async js-macro js-extern-macro structurizer rustc-cleanup contract; do
  [ -e "$W/$e" ] && ln -sfn "$W/$e" "$B/$e"
done
ln -sfn "$V/fix-bin" "$B/target"
ln -sfn "$V/fix-sysroots" "$B/build/sysroots"
