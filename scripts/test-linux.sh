#!/bin/sh
# Build and test Arcade on Linux in a container: the same three checks as on the Mac (tests,
# formatting, lint). Needs Docker running. The container's build directory is a Docker volume,
# separate from the target directory of this machine.
set -eu
cd "$(dirname "$0")/.."
if ! docker info >/dev/null 2>&1; then
  echo "Docker is not running; start it and run this again." >&2
  exit 1
fi
docker build -q -t arcade-linux-test -f docker/linux-test.Dockerfile .
exec docker run --rm -t \
  -v "$PWD":/arcade \
  -v arcade-linux-target:/cache/target \
  -v arcade-linux-cargo:/usr/local/cargo/registry \
  arcade-linux-test \
  sh -c 'cargo test --workspace && cargo fmt --all --check && cargo lint'
