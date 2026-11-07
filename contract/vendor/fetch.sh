#!/usr/bin/env bash
#
# Fetch and verify the pinned upstream sources the dom-expressions contract is
# extracted from. Run from anywhere; paths are resolved relative to this file.
#
#   ./fetch.sh
#
# Downloads into vendor/upstream/ (gitignored):
#
#   upstream/dom-expressions/                  npm tarball, unpacked
#   upstream/solid-js/                         npm tarball, unpacked
#   upstream/babel-plugin-jsx-dom-expressions/ npm tarball, unpacked
#   upstream/git/                              github codeload tarball of the
#                                              dom-expressions monorepo at the
#                                              gitHead of the 0.40.8 publish
#
# Every artifact's sha512 is verified against contract/upstream.lock. npm
# publishes integrity as base64-encoded sha512, so we compute the hex digest
# with shasum and re-encode it to base64 before comparing.
#
# The git tarball has no npm integrity field, so we pin it by commit sha (which
# github's codeload URL resolves) and additionally record the sha512 of the
# tarball bytes on first fetch. GitHub's tarballs are not byte-reproducible
# across time in principle, so a mismatch there is a warning, not an error; the
# commit sha is the real pin.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
lock="$here/../upstream.lock"
dest="$here/upstream"
tmp="$here/.fetch-tmp"

if [ ! -f "$lock" ]; then
  echo "fetch.sh: missing $lock" >&2
  exit 1
fi

# base64-encoded sha512 of a file, in the "sha512-<base64>" form npm uses.
integrity() {
  shasum -a 512 "$1" | cut -d' ' -f1 | xxd -r -p | base64 | tr -d '\n' | sed 's/^/sha512-/'
}

lookup() {
  # lookup <jq-path>
  jq -r "$1" "$lock"
}

rm -rf "$tmp"
mkdir -p "$tmp" "$dest"

fetch_npm() {
  local name="$1"
  local version bare dir url expected actual

  version="$(lookup ".npm[\"$name\"].version")"
  expected="$(lookup ".npm[\"$name\"].integrity")"
  # npm tarball URLs use the unscoped basename; the unpack dir keeps the scope
  # (flattened) so @babel/core lands in upstream/babel-core, not upstream/core.
  bare="${name##*/}"
  dir="$(printf '%s' "$name" | tr '/' '-' | sed 's/^@//')"
  url="https://registry.npmjs.org/$name/-/$bare-$version.tgz"

  echo "==> $name@$version"
  echo "    $url"
  curl -fsSL "$url" -o "$tmp/$bare.tgz"

  actual="$(integrity "$tmp/$bare.tgz")"
  if [ "$actual" != "$expected" ]; then
    echo "    INTEGRITY MISMATCH" >&2
    echo "      expected $expected" >&2
    echo "      actual   $actual" >&2
    exit 1
  fi
  echo "    integrity ok ($expected)"

  rm -rf "$dest/$dir"
  mkdir -p "$dest/$dir"
  # npm tarballs always root at package/
  tar -xzf "$tmp/$bare.tgz" -C "$dest/$dir" --strip-components=1
  echo "    unpacked -> upstream/$dir/"
}

fetch_git() {
  local repo sha url expected actual
  repo="$(lookup '.git.repo')"
  sha="$(lookup '.git.sha')"
  expected="$(lookup '.git.integrity')"
  url="https://codeload.github.com/$repo/tar.gz/$sha"

  echo "==> git $repo@$sha"
  echo "    $url"
  curl -fsSL "$url" -o "$tmp/git.tgz"

  actual="$(integrity "$tmp/git.tgz")"
  if [ "$expected" = "null" ] || [ -z "$expected" ]; then
    echo "    integrity (unpinned, record this): $actual"
  elif [ "$actual" != "$expected" ]; then
    echo "    WARNING: git tarball integrity differs from lock" >&2
    echo "      expected $expected" >&2
    echo "      actual   $actual" >&2
    echo "      (github tarballs are not guaranteed byte-stable; the commit sha" >&2
    echo "       is the authoritative pin. Verify contents before accepting.)" >&2
  else
    echo "    integrity ok ($expected)"
  fi

  rm -rf "$dest/git"
  mkdir -p "$dest/git"
  tar -xzf "$tmp/git.tgz" -C "$dest/git" --strip-components=1
  echo "    unpacked -> upstream/git/"
}

for pkg in $(jq -r '.npm | keys[]' "$lock"); do
  fetch_npm "$pkg"
done

fetch_git

rm -rf "$tmp"

echo
echo "vendor/upstream/ populated:"
ls -1 "$dest"
