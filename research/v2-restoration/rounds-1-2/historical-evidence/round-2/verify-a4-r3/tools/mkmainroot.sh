#!/usr/bin/env bash
# mkmainroot.sh <name>: a probe root for the main (ea5604f) compiler built by the first verifier
# (verify-a4/mainroot), with its own integration lock; sysroots shared read-only.
set -euo pipefail
V=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4-r3
M=/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/bf1d1602-d3a2-4fa9-88f8-ca711756aab5/scratchpad/verify-a4/mainroot
B=$V/$1
mkdir -p "$B/build"
for e in backend Cargo.toml Cargo.lock examples jsc-link patches rcgjs-object runtime rust-toolchain.toml scripts experiments mires-js view-abi view-async js-macro js-extern-macro structurizer rustc-cleanup contract target; do
  [ -e "$M/$e" ] && ln -sfn "$M/$e" "$B/$e"
done
ln -sfn "$M/build/sysroots" "$B/build/sysroots"
ls "$B" | wc -l
