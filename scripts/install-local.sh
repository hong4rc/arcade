#!/bin/sh
# Build Arcade from this checkout and install it as one file.
# usage: scripts/install-local.sh
# The binary goes to ${ARCADE_INSTALL_DIR:-$HOME/.local/bin}/arcade. This does not stop or restart
# any daemon: a running daemon keeps its old code until `arcade kill-server` and the next start.
# To go back, delete that file.
set -eu
cd "$(dirname "$0")/.."
DEST="${ARCADE_INSTALL_DIR:-$HOME/.local/bin}"
./scripts/fetch-zig.sh
./scripts/retry-build.sh cargo build --release -p arcade
mkdir -p "$DEST"
cp "${CARGO_TARGET_DIR:-target}/release/arcade" "$DEST/arcade.new"
chmod 755 "$DEST/arcade.new"
mv -f "$DEST/arcade.new" "$DEST/arcade"
echo "installed: $("$DEST/arcade" --version) at $DEST/arcade"
echo "A daemon that is already running still has the old code: run 'arcade kill-server' when you are ready, then start Arcade again."
