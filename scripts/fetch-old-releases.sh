#!/bin/sh
# Download released arcade programs into .tools/old/arcade-<version>, for the tests that run the
# restart against a daemon of a version before `arcade upgrade` existed.
# usage: scripts/fetch-old-releases.sh [<version>...]   (default: 0.1.3 0.1.4)
# Needs the GitHub command line tool; the downloads are checked against the release's SHA256SUMS.
set -eu
cd "$(dirname "$0")/.."
case "$(uname -sm)" in
  "Darwin arm64") ASSET=arcade-macos-apple-silicon ;;
  "Linux x86_64") ASSET=arcade-linux-amd64 ;;
  "Linux aarch64") ASSET=arcade-linux-arm64 ;;
  *) echo "no released build for $(uname -sm)" >&2; exit 1 ;;
esac
[ $# -gt 0 ] || set -- 0.1.3 0.1.4
mkdir -p .tools/old
for v in "$@"; do
  [ -x ".tools/old/arcade-$v" ] && continue
  work=$(mktemp -d)
  GH_TOKEN="${GH_TOKEN:-$(gh auth token --user hong4rc)}" gh release download "v$v" --repo hong4rc/arcade \
    -p "$ASSET.tar.gz" -p SHA256SUMS.txt -D "$work"
  (cd "$work" && grep " $ASSET.tar.gz\$" SHA256SUMS.txt | shasum -a 256 -c - >/dev/null)
  tar -xzf "$work/$ASSET.tar.gz" -C "$work"
  bin=$(find "$work" -type f -name arcade | head -1)
  [ -n "$bin" ] || { echo "no arcade in the $v archive" >&2; exit 1; }
  cp "$bin" ".tools/old/arcade-$v"
  chmod 755 ".tools/old/arcade-$v"
  rm -rf "$work"
  echo "fetched .tools/old/arcade-$v: $(".tools/old/arcade-$v" --version)"
done
