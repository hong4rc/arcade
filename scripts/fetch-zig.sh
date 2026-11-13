#!/bin/sh
# Download the Zig compiler that libghostty-vt needs into .tools/zig (not installed system-wide).
# Only the parser evaluation's libghostty-vt candidate needs it.
set -eu
VERSION=0.17.1
cd "$(dirname "$0")/.."
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) PLATFORM=aarch64-macos; SHA=75f1bc7a1e9852f17ec9a0c1895f767c4ec56d46bc8a834920eece1d0359dd8d ;;
  *) echo "add the tarball name and checksum for $(uname -s)-$(uname -m) from https://ziglang.org/download/" >&2; exit 1 ;;
esac
if [ -x .tools/zig/zig ] && [ "$(.tools/zig/zig version)" = "$VERSION" ]; then
  echo "zig $VERSION already present"; exit 0
fi
mkdir -p .tools
TARBALL=".tools/zig-$PLATFORM-$VERSION.tar.xz"
curl -fsSL -o "$TARBALL" "https://ziglang.org/download/$VERSION/zig-$PLATFORM-$VERSION.tar.xz"
echo "$SHA  $TARBALL" | shasum -a 256 -c -
rm -rf .tools/zig
mkdir .tools/zig
tar -xJf "$TARBALL" -C .tools/zig --strip-components 1
rm "$TARBALL"
.tools/zig/zig version
