#!/bin/sh

set -eu

RELEASES_URL=${INRO_RELEASES_URL:-https://github.com/Yangmoooo/inro/releases}
version=${INRO_VERSION:-latest}
install_dir=${INRO_INSTALL_DIR:-.}
temp_dir=
staging_dir=

usage() {
    cat <<'EOF'
Download and verify the inro binary from a GitHub Release.

Usage: install.sh [--version <version>] [--to <directory>]

Options:
  --version <version>  Download a specific version instead of the latest release
  --to <directory>     Output directory (default: current directory)
  -h, --help           Show this help

The INRO_VERSION and INRO_INSTALL_DIR environment variables provide the same
settings. Command-line options take precedence. Existing files are never replaced.
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
    if [ -n "$staging_dir" ]; then
        rm -rf "$staging_dir"
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
            [ -n "$2" ] || fail "$1 requires a non-empty directory"
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

mkdir -p "$install_dir"
install_dir=$(CDPATH= cd -- "$install_dir" && pwd -P)
target_path="$install_dir/inro"
[ ! -e "$target_path" ] && [ ! -L "$target_path" ] \
    || fail "$target_path already exists; choose another directory with --to"

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

# Stage on the destination filesystem, then publish without overwriting a file or link.
staging_dir=$(mktemp -d "$install_dir/.inro-download.XXXXXX")
cp "$extract_dir/inro" "$staging_dir/inro"
chmod 755 "$staging_dir/inro"
[ ! -d "$target_path" ] || fail "$target_path already exists as a directory"
ln -n "$staging_dir/inro" "$target_path" \
    || fail "could not create $target_path; an existing destination will not be replaced"

# Quote the actual path so these commands also work with spaces and shell metacharacters.
command_path=$(printf '%s' "$target_path" | sed "s/'/'\\\\''/g")
printf "Downloaded and verified: %s\\n\\n" "$target_path"
printf "Run directly:\\n  '%s' --help\\n\\n" "$command_path"
printf "To let inro manage itself:\\n  '%s' source update\\n  '%s' install inro\\n" "$command_path" "$command_path"
printf '\nKeep this downloaded copy outside your configured bin_dir (default ~/.local/bin) for self-management.\n'
printf 'After confirming the managed inro works, you can remove this downloaded copy.\n'
printf 'Alternatively, move it into a directory on PATH for manual use.\n'
