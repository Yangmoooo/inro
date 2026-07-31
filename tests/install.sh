#!/bin/sh

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temp_dir=$(mktemp -d)
server_pid=

cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$temp_dir"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

asset=inro-linux-x86_64-musl.tar.xz
release_dir="$temp_dir/server/releases/download/v1.2.3"
payload_dir="$temp_dir/payload"
install_dir="$temp_dir/install"
fake_bin_dir="$temp_dir/fake-bin"
mkdir -p "$release_dir" "$payload_dir" "$fake_bin_dir"

printf '#!/bin/sh\nprintf "fake-inro 1.2.3\\n"\n' > "$payload_dir/inro"
chmod +x "$payload_dir/inro"
tar -cJf "$release_dir/$asset" -C "$payload_dir" inro

printf '#!/bin/sh\ncase "$1" in\n    -s) printf "Linux\\n" ;;\n    -m) printf "x86_64\\n" ;;\nesac\n' \
    > "$fake_bin_dir/uname"
chmod +x "$fake_bin_dir/uname"

if command -v sha256sum >/dev/null 2>&1; then
    checksum=$(sha256sum "$release_dir/$asset" | awk '{ print $1 }')
else
    checksum=$(shasum -a 256 "$release_dir/$asset" | awk '{ print $1 }')
fi
printf '%s  %s\n' "$checksum" "$asset" > "$release_dir/SHA256SUMS"

port=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$temp_dir/server" \
    > "$temp_dir/server.log" 2>&1 &
server_pid=$!

attempt=0
until curl --silent --fail "http://127.0.0.1:$port/releases/download/v1.2.3/SHA256SUMS" \
    > /dev/null
do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 20 ]; then
        printf 'local release server did not start\n' >&2
        exit 1
    fi
    sleep 0.1
done

PATH="$fake_bin_dir:$PATH" INRO_RELEASES_URL="http://127.0.0.1:$port/releases" \
    "$repo_root/install.sh" --version 1.2.3 --to "$install_dir"

test -x "$install_dir/inro"
test "$("$install_dir/inro")" = "fake-inro 1.2.3"

printf '%064d  %s\n' 0 "$asset" > "$release_dir/SHA256SUMS"
if PATH="$fake_bin_dir:$PATH" INRO_RELEASES_URL="http://127.0.0.1:$port/releases" \
    "$repo_root/install.sh" --version 1.2.3 --to "$install_dir" \
    > "$temp_dir/checksum-error.log" 2>&1
then
    printf 'installer accepted an invalid checksum\n' >&2
    exit 1
fi
grep -q 'checksum verification failed' "$temp_dir/checksum-error.log"
test "$("$install_dir/inro")" = "fake-inro 1.2.3"
