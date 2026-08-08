#!/bin/sh
# Install the `deed` binary for this machine.
#
#   curl -fsSL https://raw.githubusercontent.com/deed-lang/deed/main/install.sh | sh
#
# What this does: work out which release asset fits this machine, download it
# and the release's checksum list, refuse the download if the hash does not
# match, unpack it, and put one file somewhere on your PATH. It installs into
# your home directory, so it never asks for a password and never writes outside
# it.
#
# What the hash does and does not buy: the checksums come from the same release
# as the binary, so this catches a truncated or corrupted download and does not
# catch a compromised release. Saying that plainly is better than a `sha256`
# that reads like it proves more than it does.
#
# There is no uninstaller, because there is nothing to uninstall: one file.
#
# `DEED_VERSION` pins a release, `DEED_INSTALL_DIR` says where the file goes,
# and `DEED_DOWNLOAD_BASE` points at a mirror of the release assets for a
# machine that cannot reach github.com. That last one is also how
# `crates/deed-driver/tests/install.rs` runs this script end to end against a
# release it builds itself, so the download, the hash check and the unpack are
# exercised rather than described.
#
# `crates/deed-driver/tests/install.rs` holds the platforms this knows against
# the ones `.github/workflows/release.yml` actually builds, so a platform can
# be added or dropped in one place rather than two.

set -eu

REPO="deed-lang/deed"
INSTALL_DIR="${DEED_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "this needs \`$1\` and cannot find it"
}

# One downloader, chosen once. Every fetch below goes through this so there is
# no second place where a flag like -f is remembered or forgotten.
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
    fetch_text() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
    fetch_text() { wget -qO- "$1"; }
else
    die "this needs \`curl\` or \`wget\` and cannot find either"
fi

if command -v sha256sum >/dev/null 2>&1; then
    hash_of() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    hash_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    die "this needs \`sha256sum\` or \`shasum\` to check what it downloaded, and cannot find either"
fi

need tar
need awk

# -- which machine is this ---------------------------------------------------

os="$(uname -s)"
arch="$(uname -m)"

case "$os/$arch" in
    Linux/x86_64)          target="x86_64-unknown-linux-gnu" ;;
    Darwin/arm64|Darwin/aarch64) target="aarch64-apple-darwin" ;;
    Darwin/x86_64)
        die "there is no Intel Mac build: \`cargo install deed-lang\` builds one" ;;
    *)
        die "no release is built for $os on $arch: \`cargo install deed-lang\` builds one" ;;
esac

# -- which release -----------------------------------------------------------

version="${DEED_VERSION:-${1:-}}"
if [ -z "$version" ]; then
    # One field out of the releases API. A shell is a bad JSON parser, but
    # `tag_name` is a flat string field on that object and the alternative is
    # asking the reader to install one.
    version="$(fetch_text "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name" *: *"\([^"]*\)".*/\1/p' | head -n 1)"
    [ -n "$version" ] || die "could not work out the latest release of $REPO"
fi
case "$version" in v*) ;; *) version="v$version" ;; esac

name="deed-$version-$target"
asset="$name.tar.gz"
base="${DEED_DOWNLOAD_BASE:-https://github.com/$REPO/releases/download/$version}"

# -- fetch, check, unpack ----------------------------------------------------

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $asset"
fetch "$base/$asset" "$tmp/$asset" \
    || die "could not download $base/$asset"
fetch "$base/deed-$version-checksums.txt" "$tmp/checksums.txt" \
    || die "$version publishes no checksum list, so this cannot check what it downloaded"

# The last field is the name and the first is the hash. Reading it by field
# rather than by exact spacing is what keeps this working across the several
# things `sha256sum` and `shasum` do to the middle of that line.
want="$(awk -v want="$asset" '
    { file = $NF; sub(/^[*]/, "", file); sub(/^\.\//, "", file)
      if (file == want) { print $1; exit } }' "$tmp/checksums.txt")"
[ -n "$want" ] || die "the checksum list does not mention $asset"
got="$(hash_of "$tmp/$asset")"
if [ "$want" != "$got" ]; then
    die "$asset hashes to $got and the release says $want"
fi
say "sha256 ok"

tar xzf "$tmp/$asset" -C "$tmp"
[ -f "$tmp/$name/deed" ] || die "$asset does not contain $name/deed"

mkdir -p "$INSTALL_DIR"
mv "$tmp/$name/deed" "$INSTALL_DIR/deed"
chmod +x "$INSTALL_DIR/deed"

say "installed $INSTALL_DIR/deed"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) say ""
       say "$INSTALL_DIR is not on your PATH. Add it:"
       say "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

say ""
say "next: deed new greeter && cd greeter && deed test ."
