#!/bin/sh
# Run a build command, and run it again when it fails, at most three attempts in all. Zig now and
# then crashes (a segmentation fault, then SIGABRT) the first time it fetches its dependencies into
# an empty cache; the next attempt finds the cache partly filled and goes through. A build that
# is really broken fails the same way every time, so after the last attempt this exits with the
# status of that attempt and the log above shows the compiler's own error. Use it for builds only,
# never for tests.
# usage: scripts/retry-build.sh <command> [argument...]
set -u
[ "$#" -ge 1 ] || { echo "usage: scripts/retry-build.sh <command> [argument...]" >&2; exit 2; }
MAX="${ARCADE_BUILD_ATTEMPTS:-3}"
attempt=1
while :; do
  echo "== build attempt $attempt of $MAX: $*"
  "$@" && exit 0
  status=$?
  if [ "$attempt" -ge "$MAX" ]; then
    echo "== build failed on attempt $attempt of $MAX (status $status): giving up" >&2
    exit "$status"
  fi
  echo "== build failed on attempt $attempt of $MAX (status $status): trying again" >&2
  attempt=$((attempt + 1))
done
