#!/bin/sh
# Installs the revscope CLI: downloads the release for this machine, checks its
# checksum, and puts the binary in ~/.local/bin, or in $REVSCOPE_INSTALL_DIR.
set -eu

release_url="${REVSCOPE_RELEASE_URL:-https://github.com/eloff/revscope/releases/latest/download}"
install_dir="${REVSCOPE_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    echo "revscope install: $1" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) os=unknown-linux-musl ;;
    Darwin) os=apple-darwin ;;
    *) fail "no release for $(uname -s). On Windows, use: npx revscope" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) fail "no release for $(uname -m)" ;;
esac
archive="revscope-$arch-$os.tar.gz"

download() {
    if command -v curl >/dev/null 2>&1; then
        curl --fail --silent --show-error --location --output "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget --quiet --output-document="$2" "$1"
    else
        fail "needs curl or wget"
    fi
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

download "$release_url/$archive" "$work/$archive" || fail "could not download $release_url/$archive"
download "$release_url/$archive.sha256" "$work/$archive.sha256" || fail "could not download the checksum"

expected=$(cut -d ' ' -f 1 "$work/$archive.sha256")
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$work/$archive" | cut -d ' ' -f 1)
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$work/$archive" | cut -d ' ' -f 1)
else
    fail "needs sha256sum or shasum to check the download"
fi
[ -n "$expected" ] && [ "$expected" = "$actual" ] || fail "the download's checksum doesn't match"

tar -xzf "$work/$archive" -C "$work"
mkdir -p "$install_dir"
# Moved into place in one step, so a running copy is never half overwritten.
cp "$work/revscope" "$install_dir/.revscope.$$"
chmod 755 "$install_dir/.revscope.$$"
mv -f "$install_dir/.revscope.$$" "$install_dir/revscope"

version=$("$install_dir/revscope" --version) || fail "installed to $install_dir/revscope, but it doesn't run here"
echo "Installed $version to $install_dir/revscope"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) echo "Add $install_dir to your PATH to run it as revscope." ;;
esac
