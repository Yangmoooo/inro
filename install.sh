#!/bin/sh

set -eu

RELEASES_URL=${INRO_RELEASES_URL:-https://github.com/Yangmoooo/inro/releases}
version=${INRO_VERSION:-latest}
install_dir=${INRO_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}
temp_dir=
staged_binary=

usage() {
    cat <<'EOF'
Install inro from a GitHub Release.

Usage: install.sh [--version <version>] [--to <directory>]

Options:
  --version <version>  Install a specific version instead of the latest release
  --to <directory>     Install directory (default: $HOME/.local/bin)
  -h, --help           Show this help

The INRO_VERSION and INRO_INSTALL_DIR environment variables provide the same
settings. Command-line options take precedence.
EOF
}

fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

cleanup() {
    if [ -n "$temp_dir" ]; then
        rm -rf "$temp_dir"
    fi
    if [ -n "$staged_binary" ]; then
        rm -f "$staged_binary"
    fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            version=$2
            shift 2
            ;;
        --to|--install-dir)
            [ "$#" -ge 2 ] || fail "$1 requires a value"
            install_dir=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown option: $1"
            ;;
    esac
done

case $(uname -s) in
    Linux)
        [ "$(uname -m)" = "x86_64" ] || fail "Linux releases currently require x86_64"
        asset=inro-linux-x86_64-musl.tar.xz
        ;;
    Darwin)
        case $(uname -m) in
            arm64|aarch64) asset=inro-macos-aarch64-darwin.tar.xz ;;
            *) fail "macOS releases currently require Apple silicon" ;;
        esac
        ;;
    *)
        fail "unsupported operating system; use install.ps1 on Windows or build from source"
        ;;
esac

if [ "$version" = "latest" ]; then
    release_url="$RELEASES_URL/latest/download"
    version_label=latest
else
    version=${version#v}
    case "$version" in
        ''|[!0-9]*|*[!0-9A-Za-z.+-]*) fail "invalid version: $version" ;;
    esac
    release_url="$RELEASES_URL/download/v$version"
    version_label="v$version"
fi

download() {
    url=$1
    output=$2
    if command -v curl >/dev/null 2>&1; then
        curl --proto '=https,http' --tlsv1.2 --fail --location --silent --show-error \
            --output "$output" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget --quiet --output-document="$output" "$url"
    else
        fail "curl or wget is required"
    fi
}

temp_dir=$(mktemp -d 2>/dev/null || mktemp -d -t inro-install)
archive="$temp_dir/$asset"
checksums="$temp_dir/SHA256SUMS"
extract_dir="$temp_dir/extracted"
mkdir -p "$extract_dir"

printf 'Downloading inro %s...\n' "$version_label"
download "$release_url/$asset" "$archive"
download "$release_url/SHA256SUMS" "$checksums"

expected=$(awk -v asset="$asset" '
    $2 == asset || $2 == ("*" asset) { print tolower($1); exit }
' "$checksums")
[ "${#expected}" -eq 64 ] || fail "SHA256SUMS has no valid entry for $asset"
case "$expected" in
    *[!0-9a-f]*) fail "SHA256SUMS has an invalid digest for $asset" ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$archive" | awk '{ print tolower($1) }')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$archive" | awk '{ print tolower($1) }')
else
    fail "sha256sum or shasum is required to verify the download"
fi
[ "$actual" = "$expected" ] || fail "checksum verification failed for $asset"

tar -xJf "$archive" -C "$extract_dir"
[ -f "$extract_dir/inro" ] || fail "release archive does not contain the inro binary"

mkdir -p "$install_dir"
staged_binary="$install_dir/.inro-install.$$"
cp "$extract_dir/inro" "$staged_binary"
chmod 755 "$staged_binary"
mv -f "$staged_binary" "$install_dir/inro"
staged_binary=

printf 'Installed inro to %s/inro\n' "$install_dir"
case ":${PATH:-}:" in
    *:"$install_dir":*) ;;
    *) printf 'Add %s to PATH to run inro.\n' "$install_dir" ;;
esac
