#!/bin/sh
# Check, build and (only with --publish) publish a release from this machine, with no CI run.
# usage: scripts/release-local.sh <version> [--skip-linux-tests] [--publish]
#   check      clean tree, on main, Cargo.toml version equals <version>, tag v<version> absent here
#              and on the remote, then tests, format and lint on the Mac and in Linux (Docker).
#   build      dist/arcade-macos-apple-silicon.tar.gz and dist/arcade-linux-arm64.tar.gz (the Linux
#              one is built in a container on the same image as test-linux.sh), each binary checked
#              to print `arcade <version>` and each archive checked to keep the executable bit.
#   checksums  dist/SHA256SUMS.txt
#   --publish  push main and the tag, create the GitHub release with the archives and checksums,
#              as the hong4rc account (the active gh account is never switched).
# Without --publish nothing leaves this machine. The Homebrew tap commit is printed, not run.
set -eu
cd "$(dirname "$0")/.."

VERSION=""; SKIP_LINUX=0; PUBLISH=0
for arg in "$@"; do
  case "$arg" in
    --skip-linux-tests) SKIP_LINUX=1 ;;
    --publish) PUBLISH=1 ;;
    -*) echo "unknown option $arg" >&2; exit 2 ;;
    *) VERSION="${arg#v}" ;;
  esac
done
[ -n "$VERSION" ] || { sed -n '2,13p' "$0" >&2; exit 2; }
TAG="v$VERSION"
fail() { echo "release: $*" >&2; exit 1; }

echo "== check"
[ -z "$(git status --porcelain)" ] || fail "the working tree is not clean"
[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || fail "not on main"
CARGO_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ "$CARGO_VERSION" = "$VERSION" ] || fail "Cargo.toml says $CARGO_VERSION, not $VERSION"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && fail "tag $TAG already exists here"
REMOTE_TAG=$(git ls-remote --tags origin "refs/tags/$TAG") || fail "cannot reach the remote"
[ -z "$REMOTE_TAG" ] || fail "tag $TAG already exists on the remote"
cargo test --workspace
cargo fmt --all --check
cargo lint
if [ "$SKIP_LINUX" = 0 ]; then
  docker info >/dev/null 2>&1 || fail "Docker is not running (start it, or pass --skip-linux-tests)"
  ./scripts/test-linux.sh
else
  echo "(Linux tests skipped on request)"
fi

echo "== build"
docker info >/dev/null 2>&1 || fail "Docker is not running: the Linux archive is built in a container"
rm -rf dist
./scripts/package.sh macos-apple-silicon
docker build -q -t arcade-linux-test -f docker/linux-test.Dockerfile . >/dev/null
docker run --rm \
  -v "$PWD":/arcade \
  -v arcade-linux-release-target:/cache/target \
  -v arcade-linux-cargo:/usr/local/cargo/registry \
  arcade-linux-test \
  sh -c 'ARCADE_TOOLS_DIR=/opt/tools ./scripts/package.sh linux-arm64'
for target in macos-apple-silicon linux-arm64; do
  archive="dist/arcade-$target.tar.gz"
  [ -f "$archive" ] || fail "$archive was not built"
  tar -tvzf "$archive" | grep -Eq "^-rwx.* arcade-$target/arcade\$" || fail "$archive lost the executable bit"
done
tmp=$(mktemp -d)
tar -C "$tmp" -xzf dist/arcade-macos-apple-silicon.tar.gz
[ "$("$tmp/arcade-macos-apple-silicon/arcade" --version)" = "arcade $VERSION" ] || fail "the macOS binary does not print arcade $VERSION"
rm -rf "$tmp"
tmp=$(mktemp -d)
tar -C "$tmp" -xzf dist/arcade-linux-arm64.tar.gz
LINUX_VERSION=$(docker run --rm -v "$tmp":/x:ro arcade-linux-test /x/arcade-linux-arm64/arcade --version)
rm -rf "$tmp"
[ "$LINUX_VERSION" = "arcade $VERSION" ] || fail "the Linux binary prints '$LINUX_VERSION', not arcade $VERSION"

echo "== checksums"
(cd dist && shasum -a 256 arcade-macos-apple-silicon.tar.gz arcade-linux-arm64.tar.gz > SHA256SUMS.txt)
cat dist/SHA256SUMS.txt

if [ "$PUBLISH" = 1 ]; then
  echo "== publish"
  export GH_TOKEN="$(gh auth token --user hong4rc)"
  git tag "$TAG"
  git push origin main "$TAG"
  gh release create "$TAG" dist/arcade-macos-apple-silicon.tar.gz dist/arcade-linux-arm64.tar.gz \
    dist/SHA256SUMS.txt --repo hong4rc/arcade --title "$TAG" --generate-notes
else
  echo "== not published (pass --publish to push main and $TAG and create the release)"
fi

cat <<NOTE

== Homebrew formula (run by hand in a checkout of hong4rc/homebrew-tap, as hong4rc):
   python3 /path/to/arcade/scripts/bump-formula.py Formula/arcade.rb $TAG /path/to/arcade/dist/SHA256SUMS.txt
   git commit -am "arcade $VERSION" && git push
   (a target missing from SHA256SUMS.txt, here linux-amd64, keeps its old url and sha256)
NOTE
