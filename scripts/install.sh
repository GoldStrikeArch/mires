#!/bin/sh
# POSIX shell installer for cargo-mires (macOS and Linux).
#
# Usage:
#   curl -fsSL https://github.com/GoldStrikeArch/mires/releases/latest/download/install.sh | sh
#
# Environment variables:
#   MIRES_VERSION      Release version to install (default: latest; accepts "0.1.0" or "v0.1.0")
#   MIRES_INSTALL_DIR  Destination directory (default: ~/.cargo/bin if ~/.cargo exists, else ~/.local/bin)
#   MIRES_SKIP_SETUP   Set to 1 to skip running `cargo-mires setup` after installation
#   MIRES_RELEASE_URL  Override release download base URL (for mirrors or local testing)

set -eu

REPO_URL="https://github.com/GoldStrikeArch/mires"
API_LATEST_URL="https://api.github.com/repos/GoldStrikeArch/mires/releases/latest"

TMPDIR_WORK=""
STAGED_BIN=""

say() {
    printf '%s\n' "$*"
}

err() {
    printf 'error: %s\n' "$*" >&2
}

die() {
    err "$*"
    exit 1
}

cleanup() {
    if [ -n "$STAGED_BIN" ] && [ -e "$STAGED_BIN" ]; then
        rm -f "$STAGED_BIN"
    fi
    if [ -n "$TMPDIR_WORK" ] && [ -d "$TMPDIR_WORK" ]; then
        rm -rf "$TMPDIR_WORK"
    fi
}

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        die "required command '$1' was not found on PATH"
    fi
}

detect_downloader() {
    if command -v curl >/dev/null 2>&1; then
        printf '%s\n' "curl"
    elif command -v wget >/dev/null 2>&1; then
        printf '%s\n' "wget"
    else
        die "either 'curl' or 'wget' is required to download cargo-mires"
    fi
}

download_file() {
    dl_tool="$1"
    dl_url="$2"
    dl_out="$3"

    if [ "$dl_tool" = "curl" ]; then
        curl -fsSL --retry 3 --retry-delay 1 -o "$dl_out" "$dl_url"
    else
        wget -q -O "$dl_out" "$dl_url"
    fi
}

resolve_latest_tag() {
    dl_tool="$1"
    resolved_tag=""

    if [ "$dl_tool" = "curl" ]; then
        effective_url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "${REPO_URL}/releases/latest" 2>/dev/null || true)
        case "$effective_url" in
            */releases/tag/*)
                resolved_tag="${effective_url##*/releases/tag/}"
                resolved_tag="${resolved_tag%%\?*}"
                resolved_tag="${resolved_tag%%#*}"
                ;;
        esac
    fi

    if [ -z "$resolved_tag" ]; then
        api_json="$TMPDIR_WORK/latest.json"
        if [ "$dl_tool" = "curl" ]; then
            curl -fsSL -H "User-Agent: mires-installer" -o "$api_json" "$API_LATEST_URL"
        else
            wget -q --header="User-Agent: mires-installer" -O "$api_json" "$API_LATEST_URL"
        fi
        resolved_tag=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$api_json" | head -n 1)
    fi

    if [ -z "$resolved_tag" ]; then
        die "could not determine latest mires release tag; set MIRES_VERSION explicitly"
    fi

    printf '%s\n' "$resolved_tag"
}

detect_target() {
    raw_os=$(uname -s)
    raw_arch=$(uname -m)

    case "$raw_os" in
        Darwin) target_os="apple-darwin" ;;
        Linux)  target_os="unknown-linux-gnu" ;;
        *)
            die "unsupported operating system '$raw_os' (supported: Darwin, Linux)"
            ;;
    esac

    case "$raw_arch" in
        arm64|aarch64)       target_arch="aarch64" ;;
        x86_64|amd64|x64)    target_arch="x86_64" ;;
        *)
            die "unsupported CPU architecture '$raw_arch' (supported: arm64/aarch64, x86_64/amd64)"
            ;;
    esac

    printf '%s-%s\n' "$target_arch" "$target_os"
}

compute_sha256() {
    file_to_hash="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file_to_hash" | awk '{ print tolower($1) }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file_to_hash" | awk '{ print tolower($1) }'
    else
        die "neither 'sha256sum' nor 'shasum' is available to verify SHA-256 checksums"
    fi
}

lookup_expected_sha256() {
    sums_file="$1"
    asset_name="$2"

    awk -v file="$asset_name" '
        {
            if (NF < 2) next
            name = $2
            sub(/^\*/, "", name)
            sub(/^\.\//, "", name)
            if (name == file) {
                print tolower($1)
                exit 0
            }
        }
    ' "$sums_file"
}

infer_version_from_sums() {
    sums_file="$1"
    target_triple="$2"

    awk -v target="$target_triple" '
        {
            if (NF < 2) next
            name = $2
            sub(/^\*/, "", name)
            sub(/^\.\//, "", name)
            prefix = "cargo-mires-v"
            suffix = "-" target ".tar.gz"
            if (index(name, prefix) == 1 && length(name) > length(prefix) + length(suffix)) {
                tail = substr(name, length(name) - length(suffix) + 1)
                if (tail == suffix) {
                    print substr(name, length(prefix) + 1, length(name) - length(prefix) - length(suffix))
                    exit 0
                }
            }
        }
    ' "$sums_file"
}

verify_sha256() {
    sums_file="$1"
    asset_name="$2"
    asset_path="$3"

    expected_sha=$(lookup_expected_sha256 "$sums_file" "$asset_name")
    if [ -z "$expected_sha" ]; then
        die "checksum for '$asset_name' not found in SHA256SUMS"
    fi
    case "$expected_sha" in
        *[!0-9a-f]*)
            die "invalid SHA-256 hex digest '$expected_sha' for '$asset_name' in SHA256SUMS"
            ;;
    esac
    if [ "${#expected_sha}" -ne 64 ]; then
        die "invalid SHA-256 digest length for '$asset_name' in SHA256SUMS"
    fi

    actual_sha=$(compute_sha256 "$asset_path")
    if [ "$actual_sha" != "$expected_sha" ]; then
        err "SHA-256 checksum mismatch for $asset_name"
        err "  expected: $expected_sha"
        err "  actual:   $actual_sha"
        exit 1
    fi
}

main() {
    trap cleanup EXIT HUP INT TERM

    need_cmd uname
    need_cmd tar
    need_cmd awk
    need_cmd mktemp
    need_cmd chmod
    need_cmd mv
    need_cmd cp

    downloader=$(detect_downloader)
    target=$(detect_target)

    TMPDIR_WORK=$(mktemp -d "${TMPDIR:-/tmp}/mires-install.XXXXXX")

    requested_version="${MIRES_VERSION:-latest}"
    release_override="${MIRES_RELEASE_URL:-}"
    checksum_path="$TMPDIR_WORK/SHA256SUMS"

    if [ -n "$release_override" ]; then
        base_url="${release_override%/}"
        if [ "$requested_version" = "latest" ] || [ -z "$requested_version" ]; then
            say "Downloading SHA256SUMS from ${base_url}..."
            download_file "$downloader" "${base_url}/SHA256SUMS" "$checksum_path"
            version=$(infer_version_from_sums "$checksum_path" "$target")
            if [ -z "$version" ]; then
                die "could not infer version for '$target' from ${base_url}/SHA256SUMS; set MIRES_VERSION explicitly"
            fi
        else
            version="${requested_version#v}"
            case "$base_url" in
                *"/v${version}")
                    say "Downloading SHA256SUMS from ${base_url}..."
                    download_file "$downloader" "${base_url}/SHA256SUMS" "$checksum_path"
                    ;;
                *)
                    if download_file "$downloader" "${base_url}/v${version}/SHA256SUMS" "$checksum_path" 2>/dev/null; then
                        base_url="${base_url}/v${version}"
                    else
                        say "Downloading SHA256SUMS from ${base_url}..."
                        download_file "$downloader" "${base_url}/SHA256SUMS" "$checksum_path"
                    fi
                    ;;
            esac
        fi
    else
        if [ "$requested_version" = "latest" ] || [ -z "$requested_version" ]; then
            tag=$(resolve_latest_tag "$downloader")
            version="${tag#v}"
        else
            version="${requested_version#v}"
            tag="v${version}"
        fi
        base_url="${REPO_URL}/releases/download/${tag}"
        say "Downloading SHA256SUMS for cargo-mires v${version}..."
        download_file "$downloader" "${base_url}/SHA256SUMS" "$checksum_path"
    fi

    if [ -z "$version" ]; then
        die "resolved version is empty"
    fi

    archive_name="cargo-mires-v${version}-${target}.tar.gz"
    archive_path="$TMPDIR_WORK/$archive_name"

    say "Downloading ${archive_name}..."
    download_file "$downloader" "${base_url}/${archive_name}" "$archive_path"

    say "Verifying SHA-256 checksum..."
    verify_sha256 "$checksum_path" "$archive_name" "$archive_path"

    extract_dir="$TMPDIR_WORK/extract"
    mkdir -p "$extract_dir"
    tar -xzf "$archive_path" -C "$extract_dir"

    extracted_bin="$extract_dir/cargo-mires"
    if [ ! -f "$extracted_bin" ]; then
        die "archive '$archive_name' did not contain expected 'cargo-mires' binary at archive root"
    fi

    if [ -n "${MIRES_INSTALL_DIR:-}" ]; then
        install_dir="$MIRES_INSTALL_DIR"
    elif [ -d "${HOME}/.cargo" ]; then
        install_dir="${HOME}/.cargo/bin"
    else
        install_dir="${HOME}/.local/bin"
    fi

    mkdir -p "$install_dir"
    STAGED_BIN="$install_dir/.cargo-mires.tmp.$$"
    cp "$extracted_bin" "$STAGED_BIN"
    chmod 0755 "$STAGED_BIN"
    mv -f "$STAGED_BIN" "$install_dir/cargo-mires"
    STAGED_BIN=""

    say "Installed cargo-mires v${version} to ${install_dir}/cargo-mires"

    case ":${PATH:-}:" in
        *":${install_dir}:"*) ;;
        *)
            say ""
            say "Note: ${install_dir} is not currently in your PATH."
            say "Add the following line to your shell profile (~/.zshrc, ~/.bashrc, or ~/.profile):"
            say "  export PATH=\"${install_dir}:\$PATH\""
            ;;
    esac

    if [ "${MIRES_SKIP_SETUP:-0}" = "1" ]; then
        say ""
        say "Skipping toolchain setup (MIRES_SKIP_SETUP=1)."
        say "Run '${install_dir}/cargo-mires setup' when ready to download the compiler toolchain."
    else
        say ""
        say "Running 'cargo-mires setup' to install the prebuilt compiler toolchain..."
        if [ -n "$release_override" ]; then
            "$install_dir/cargo-mires" setup --version "$version" --url "$base_url"
        else
            "$install_dir/cargo-mires" setup --version "$version"
        fi
    fi
}

main "$@"
