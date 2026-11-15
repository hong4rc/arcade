#!/bin/sh
# Download the Zig compiler that libghostty-vt needs into .tools/zig (not installed system-wide).
# Set ARCADE_TOOLS_DIR to put it somewhere else (the Linux test container does, so that it does not
# write over the macOS compiler in the mounted source).
set -eu
VERSION=0.17.1
cd "$(dirname "$0")/.."
TOOLS="${ARCADE_TOOLS_DIR:-.tools}"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) PLATFORM=aarch64-macos; SHA=75f1bc7a1e9852f17ec9a0c1895f767c4ec56d46bc8a834920eece1d0359dd8d ;;
  Darwin-x86_64) PLATFORM=x86_64-macos; SHA=9af07b20319fbd0c92ee46ad23e81d6a782b1aaea2cfb8dcbff35f1d2c7ddbb7 ;;
  Linux-aarch64) PLATFORM=aarch64-linux; SHA=a1546d5646eb184437918d6635ca6b762a0e889eac09cef720ee7a1ed3fe7795 ;;
  Linux-x86_64) PLATFORM=x86_64-linux; SHA=3581863e3cbcfd0fce0efc682ce7e9e93da96668a977791d356a26d2a7e9b17a ;;
  *) echo "add the tarball name and checksum for $(uname -s)-$(uname -m) from https://ziglang.org/download/" >&2; exit 1 ;;
esac
if [ -x "$TOOLS/zig/zig" ] && [ "$("$TOOLS/zig/zig" version)" = "$VERSION" ]; then
  echo "zig $VERSION already present"; exit 0
fi
mkdir -p "$TOOLS"
TARBALL="$TOOLS/zig-$PLATFORM-$VERSION.tar.xz"
curl -fsSL -o "$TARBALL" "https://ziglang.org/download/$VERSION/zig-$PLATFORM-$VERSION.tar.xz"
if command -v sha256sum >/dev/null 2>&1; then
  echo "$SHA  $TARBALL" | sha256sum -c -
else
  echo "$SHA  $TARBALL" | shasum -a 256 -c -
fi
rm -rf "$TOOLS/zig"
mkdir "$TOOLS/zig"
tar -xJf "$TARBALL" -C "$TOOLS/zig" --strip-components 1
rm "$TARBALL"
"$TOOLS/zig/zig" version
