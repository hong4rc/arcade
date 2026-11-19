#!/bin/sh
# Download the cargo-nextest test runner into .tools/nextest (not installed system-wide).
# Set ARCADE_TOOLS_DIR to put it somewhere else, as for scripts/fetch-zig.sh.
set -eu
VERSION=0.9.152
cd "$(dirname "$0")/.."
TOOLS="${ARCADE_TOOLS_DIR:-.tools}"
case "$(uname -s)-$(uname -m)" in
  Darwin-*) PLATFORM=universal-apple-darwin; SHA=2fadf57df4243e691f088ee3bff4f4ab9af16da0c95a1db84b5cb1685e098a76 ;;
  Linux-x86_64) PLATFORM=x86_64-unknown-linux-gnu; SHA=d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a ;;
  Linux-aarch64) PLATFORM=aarch64-unknown-linux-gnu; SHA=ca1f81a5b86533eb45ab97e0064aa88e925e53aab9a698ca6331a7cad1f85f07 ;;
  *) echo "add the archive name and checksum for $(uname -s)-$(uname -m) from https://github.com/nextest-rs/nextest/releases" >&2; exit 1 ;;
esac
if [ -x "$TOOLS/nextest/cargo-nextest" ] && "$TOOLS/nextest/cargo-nextest" nextest --version 2>/dev/null | grep -q "cargo-nextest $VERSION"; then
  echo "nextest $VERSION already present"; exit 0
fi
mkdir -p "$TOOLS"
ARCHIVE="$TOOLS/cargo-nextest-$VERSION-$PLATFORM.tar.gz"
curl -fsSL -o "$ARCHIVE" "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$VERSION/cargo-nextest-$VERSION-$PLATFORM.tar.gz"
if command -v sha256sum >/dev/null 2>&1; then
  echo "$SHA  $ARCHIVE" | sha256sum -c -
else
  echo "$SHA  $ARCHIVE" | shasum -a 256 -c -
fi
rm -rf "$TOOLS/nextest"
mkdir "$TOOLS/nextest"
tar -xzf "$ARCHIVE" -C "$TOOLS/nextest"
rm "$ARCHIVE"
"$TOOLS/nextest/cargo-nextest" nextest --version | head -1
