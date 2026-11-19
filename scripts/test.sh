#!/usr/bin/env bash
# Runs the whole test suite: every test through cargo-nextest, then the doc tests
# (nextest does not run those). Exits non-zero when anything fails.
# Extra arguments go to nextest, for example: scripts/test.sh -E 'test(notice)'
set -euo pipefail
cd "$(dirname "$0")/.."

nextest=.tools/nextest/cargo-nextest
[ -x "$nextest" ] || ./scripts/fetch-nextest.sh

"$nextest" nextest run --workspace --no-fail-fast "$@"
cargo test --doc --workspace
