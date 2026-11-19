#!/usr/bin/env bash
# Installs the pinned GopherJS comparator toolchain under .toolchains/go without
# touching the system Go, GOPATH or shell startup files.
#
# GopherJS 1.21.0 compiles Go 1.21 code and needs a Go 1.21 distribution for its
# standard library sources (GOPHERJS_GOROOT); a newer system Go is not a
# substitute. The Go archive is verified against the SHA-256 published on
# go.dev/dl before it is unpacked; GopherJS and its module dependencies are
# verified by the Go checksum database.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
base=${MIRES_GO_ROOT:-"$root/.toolchains/go"}
go_version=1.21.13
gopherjs_version=v1.21.0

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) archive=go$go_version.darwin-arm64.tar.gz; sha=c04ee7bdc0e65cf17133994c40ee9bdfa1b1dc9587b3baedaea39affdb8e5b49 ;;
  Darwin-x86_64) archive=go$go_version.darwin-amd64.tar.gz; sha=796fd05e8741f6776c505eb201922864f2e32991679b639d9fcb524dbe300c0d ;;
  Linux-aarch64|Linux-arm64) archive=go$go_version.linux-arm64.tar.gz; sha=2ca2d70dc9c84feef959eb31f2a5aac33eefd8c97fe48f1548886d737bffabd4 ;;
  Linux-x86_64) archive=go$go_version.linux-amd64.tar.gz; sha=502fc16d5910562461e6a6631fb6377de2322aad7304bf2bcd23500ba9dab4a7 ;;
  *) echo "unsupported host: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

goroot="$base/go$go_version"
# Downloads are retried: proxy.golang.org and go.dev occasionally reset connections.
retry() {
  local attempt
  for attempt in 1 2 3; do
    "$@" && return 0
    echo "attempt $attempt failed: $*" >&2
    sleep $((attempt * 5))
  done
  return 1
}
mkdir -p "$base/downloads" "$base/bin" "$base/gopath"
if [ ! -x "$goroot/bin/go" ]; then
  download="$base/downloads/$archive"
  [ -f "$download" ] || retry curl -fsSL -o "$download.part" "https://go.dev/dl/$archive"
  [ -f "$download" ] || mv "$download.part" "$download"
  actual=$(shasum -a 256 "$download" | cut -d' ' -f1)
  if [ "$actual" != "$sha" ]; then
    echo "checksum mismatch for $archive: $actual" >&2
    rm -f "$download"
    exit 1
  fi
  staging="$base/staging.$$"
  rm -rf "$staging" && mkdir -p "$staging"
  tar -xzf "$download" -C "$staging"
  mv "$staging/go" "$goroot"
  rmdir "$staging"
fi

export GOROOT="$goroot" GOPATH="$base/gopath" GOBIN="$base/bin" GOCACHE="$base/cache" GOTOOLCHAIN=local GOFLAGS=-modcacherw
export PATH="$goroot/bin:$PATH"
if ! GOPHERJS_GOROOT="$goroot" "$base/bin/gopherjs" version 2>/dev/null | grep -q "GopherJS ${gopherjs_version#v}"; then
  if [ "$(uname -s)" = Darwin ]; then
    # Go 1.21's internal linker omits LC_UUID. Current macOS dyld refuses such a
    # binary when it links system frameworks ("missing LC_UUID load command"),
    # and GopherJS links CoreFoundation and Security; a pure-Go binary still
    # runs. Go fixed this only from 1.22. Link with the system linker instead,
    # then re-sign: Go rewrites the binary after the linker's ad-hoc signature,
    # and the kernel kills a binary whose signature no longer matches.
    retry go install -ldflags=-linkmode=external "github.com/gopherjs/gopherjs@$gopherjs_version"
    codesign -f -s - "$base/bin/gopherjs"
  else
    retry go install "github.com/gopherjs/gopherjs@$gopherjs_version"
  fi
fi

go version
GOPHERJS_GOROOT="$goroot" "$base/bin/gopherjs" version
