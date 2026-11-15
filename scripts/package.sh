#!/bin/sh
# Build the release binary and pack it as dist/arcade-<target>.tar.gz, which holds a directory
# arcade-<target>/ with `arcade`, README.md and LICENSE (the archive keeps the executable bit).
# usage: scripts/package.sh <macos-apple-silicon|macos-intel|linux-amd64|linux-arm64>
# The target must be the machine this runs on: nothing is cross-compiled.
set -eu
cd "$(dirname "$0")/.."
TARGET="${1:?usage: scripts/package.sh <macos-apple-silicon|macos-intel|linux-amd64|linux-arm64>}"
case "$TARGET" in
  macos-apple-silicon) HOST=Darwin-arm64 ;;
  macos-intel) HOST=Darwin-x86_64 ;;
  linux-amd64) HOST=Linux-x86_64 ;;
  linux-arm64) HOST=Linux-aarch64 ;;
  *) echo "unknown target $TARGET" >&2; exit 2 ;;
esac
if [ "$(uname -s)-$(uname -m)" != "$HOST" ]; then
  echo "target $TARGET must be built on $HOST, this is $(uname -s)-$(uname -m)" >&2
  exit 1
fi
./scripts/fetch-zig.sh
cargo build --release -p arcade
NAME="arcade-$TARGET"
rm -rf "dist/$NAME" "dist/$NAME.tar.gz"
mkdir -p "dist/$NAME"
cp target/release/arcade README.md LICENSE "dist/$NAME/"
chmod 755 "dist/$NAME/arcade"
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
rm -rf "dist/$NAME"
echo "dist/$NAME.tar.gz"
