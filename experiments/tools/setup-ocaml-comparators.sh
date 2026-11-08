#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
export OPAMROOT=${MIRES_OPAM_ROOT:-"$root/.toolchains/opam"}
switch=${MIRES_OPAM_SWITCH:-mires-comparators}
jobs=${MIRES_BUILD_JOBS:-4}
# Dune auto-detection crashes in this macOS environment; an explicit limit avoids it.
export DUNE_JOBS=${DUNE_JOBS:-$jobs}
command -v opam >/dev/null || { echo 'Install opam first (macOS: brew install opam)' >&2; exit 1; }
if [ ! -f "$OPAMROOT/config" ]; then
  opam init --bare --no-setup --yes default https://opam.ocaml.org
fi
if ! opam switch list --short | grep -Fxq "$switch"; then
  opam switch create "$switch" ocaml-base-compiler.5.3.0 --no-switch --yes --jobs="$jobs"
fi
if [ -f "$root/experiments/ocaml-comparators.export" ]; then
  opam switch import "$root/experiments/ocaml-comparators.export" --switch="$switch" --yes --jobs="$jobs"
else
  opam install --switch="$switch" --yes --jobs="$jobs" \
    dune.3.24.2 melange.7.0.1-53 js_of_ocaml.6.4.1 js_of_ocaml-ppx.6.4.1 lwt.6.1.2
fi
opam list --switch="$switch" --installed --columns=name,version
