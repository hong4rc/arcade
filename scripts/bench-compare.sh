#!/bin/bash
# Compare two versions of Arcade on the benchmark suite, quickly and the same way every time.
#
#   scripts/bench-compare.sh <commit-or-path-A> <commit-or-path-B> [--full] [--config-b <toml>] [--force]
#
# A and B are each a commit (anything `git rev-parse` knows) or a directory (a working tree, with
# its uncommitted and untracked files). Each is built in release mode in one working directory
# outside the repository (so the slow libghostty part is built once and shared), the programs
# are kept per version (an unchanged version is not built again), and the suite runs
# interleaved A, B, A, B, A, B on its own sockets: never the default daemon, never the default
# tmux. The default profile is short (plain, colored and fullscreen throughput, and the key
# latency idle and while another pane floods); `--full` runs every load at the full sizes.
# `--config-b <toml>` gives B that configuration file (A gets an empty one) and makes both sides
# start with one pane that holds an `input` notification, so a setting that changes how a tab
# with a state is drawn is the only difference.
#
# It prints one table (median, spread, difference) and writes it to bench-results/. The exit
# status is 1 when B is slower than A by more than 2 percent on a throughput load or on the
# median key latency, 2 when the run was stopped by the machine check, 0 otherwise.
set -eu

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CACHE="${ARCADE_BENCH_CACHE:-$HOME/.cache/arcade-bench}"
FULL=0
FORCE=0
CONFIG_B=""
POSITIONAL=()
while [ $# -gt 0 ]; do
  case "$1" in
    --full) FULL=1 ;;
    --force) FORCE=1 ;;
    --config-b) shift; CONFIG_B="${1:?--config-b needs a file}" ;;
    -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
    -*) echo "unknown option $1" >&2; exit 64 ;;
    *) POSITIONAL+=("$1") ;;
  esac
  shift
done
[ "${#POSITIONAL[@]}" = 2 ] || { sed -n '2,5p' "$0" >&2; exit 64; }
A_SPEC="${POSITIONAL[0]}"
B_SPEC="${POSITIONAL[1]}"
[ -z "$CONFIG_B" ] || [ -f "$CONFIG_B" ] || { echo "no such file: $CONFIG_B" >&2; exit 64; }
[ -x "$REPO/.tools/zig/zig" ] || { echo "run ./scripts/fetch-zig.sh first" >&2; exit 64; }

# ---- The machine ----------------------------------------------------------------------------
busy=""
if pgrep -x cargo >/dev/null 2>&1 || pgrep -x rustc >/dev/null 2>&1; then
  busy="cargo or rustc is running: $( (pgrep -x cargo; pgrep -x rustc) 2>/dev/null | tr '\n' ' ')"
fi
cores="$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4)"
load="$( (sysctl -n vm.loadavg 2>/dev/null | tr -d '{}' || cut -d' ' -f1-3 /proc/loadavg) | awk '{print $1}')"
if awk -v l="$load" -v c="$cores" 'BEGIN { exit !(l > c * 0.3) }'; then
  busy="${busy:+$busy; }the load average is $load on $cores cores"
fi
if [ -n "$busy" ] && [ "$FORCE" = 0 ]; then
  echo "The machine is not quiet: $busy." >&2
  echo "Numbers taken now wobble. Stop what runs, or go on anyway." >&2
  if [ -t 0 ]; then
    printf 'Continue? [y/N] ' >&2
    read -r answer
    case "$answer" in y|Y|yes) ;; *) exit 2 ;; esac
  else
    echo "(not a terminal: use --force to go on)" >&2
    exit 2
  fi
fi

# ---- Making a version -----------------------------------------------------------------------
mkdir -p "$CACHE/bin" "$CACHE/work" "$CACHE/stage"
WORK="$CACHE/work"
export CARGO_TARGET_DIR="$CACHE/target"
START_ALL=$(date +%s)

# key_of <spec>: a name that stands for the contents of the version.
key_of() {
  if [ -d "$1" ]; then
    (cd "$1" && { git ls-files -z -c -o --exclude-standard | xargs -0 shasum 2>/dev/null; } | shasum | cut -c1-12 | sed 's/^/tree-/')
  else
    git -C "$REPO" rev-parse --verify --short=12 "$1^{commit}"
  fi
}

# stage_of <spec>: put the sources of the version in the working directory.
stage_sources() {
  rm -rf "$CACHE/stage" && mkdir -p "$CACHE/stage"
  if [ -d "$1" ]; then
    (cd "$1" && git ls-files -z -c -o --exclude-standard | rsync -a --from0 --files-from=- ./ "$CACHE/stage/")
  else
    git -C "$REPO" archive "$1" | tar -x -C "$CACHE/stage"
  fi
  # A copy keeps the modification times of the archive (the commit time), which are older than the
  # last build, so cargo would take a changed file for an unchanged one: touch every source file.
  # (The libghostty part is cached by the Zig cache, so this costs the workspace crates only.)
  rsync -a --delete --exclude target --exclude .tools "$CACHE/stage/" "$WORK/"
  find "$WORK" \( -path "$WORK/target" -o -path "$WORK/.tools" \) -prune -o -type f -exec touch {} +
  [ -e "$WORK/.tools" ] || ln -s "$REPO/.tools" "$WORK/.tools"
}

# build <spec> <key>: release programs of the version in $CACHE/bin/<key>/, built when absent.
build() {
  local spec="$1" key="$2"
  if [ -x "$CACHE/bin/$key/arcade" ] && [ -x "$CACHE/bin/$key/arcade-bench" ]; then
    echo "build $key: kept from before"
    return
  fi
  local t0
  t0=$(date +%s)
  stage_sources "$spec"
  (cd "$WORK" && cargo build --release -q -p arcade -p arcade-bench)
  mkdir -p "$CACHE/bin/$key"
  /bin/cp -f "$CARGO_TARGET_DIR/release/arcade" "$CARGO_TARGET_DIR/release/arcade-bench" "$CACHE/bin/$key/"
  echo "build $key: $(( $(date +%s) - t0 )) s"
}

A_KEY="$(key_of "$A_SPEC")"
B_KEY="$(key_of "$B_SPEC")"
build "$A_SPEC" "$A_KEY"
build "$B_SPEC" "$B_KEY"
if [ "$A_KEY" != "$B_KEY" ] && cmp -s "$CACHE/bin/$A_KEY/arcade" "$CACHE/bin/$B_KEY/arcade"; then
  echo "The two versions built to the same program: nothing to compare (delete $CACHE/bin/$B_KEY and run again if that is a mistake)." >&2
  exit 3
fi

# ---- Running ---------------------------------------------------------------------------------
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT="$(mktemp -d "${TMPDIR:-/tmp}/arcade-compare.XXXXXX")"
EMPTY="$OUT/empty.toml"
: > "$EMPTY"
cleanup() { pkill -TERM -f "$CACHE/bin/.*/arcade-bench" 2>/dev/null || true; }
trap cleanup INT TERM

run_one() { # run_one <side> <key> <n>
  local side="$1" key="$2" n="$3" dir config t0
  dir="$OUT/$side-$n"
  mkdir -p "$dir"
  config="$EMPTY"
  [ "$side" = B ] && [ -n "$CONFIG_B" ] && config="$CONFIG_B"
  local env_notify=()
  [ -n "$CONFIG_B" ] && env_notify=(ARCADE_BENCH_NOTIFY=1)
  t0=$(date +%s)
  if [ "$FULL" = 1 ]; then
    (cd "$REPO" && env ${env_notify[@]+"${env_notify[@]}"} ARCADE_BENCH_EXE="$CACHE/bin/$key/arcade" ARCADE_CONFIG="$config" \
      ARCADE_BENCH_OUT="$dir" "$CACHE/bin/$B_KEY/arcade-bench" --subjects arcade --tests throughput,latency >"$dir/log.txt" 2>&1)
  else
    (cd "$REPO" && env ${env_notify[@]+"${env_notify[@]}"} ARCADE_BENCH_LOADS=plain,colored,fullscreen ARCADE_BENCH_EXE="$CACHE/bin/$key/arcade" \
      ARCADE_CONFIG="$config" ARCADE_BENCH_OUT="$dir" "$CACHE/bin/$B_KEY/arcade-bench" --quick --subjects arcade \
      --tests throughput,latency >"$dir/log.txt" 2>&1)
  fi
  echo "run $side $n: $(( $(date +%s) - t0 )) s"
}

for n in 1 2 3; do
  run_one A "$A_KEY" "$n"
  run_one B "$B_KEY" "$n"
done

# ---- The table -------------------------------------------------------------------------------
mkdir -p "$REPO/bench-results"
TABLE="$REPO/bench-results/compare-$A_KEY-$B_KEY-$STAMP.txt"
set +e
python3 - "$OUT" "$A_SPEC ($A_KEY)" "$B_SPEC ($B_KEY)" "$CONFIG_B" "$FULL" > "$TABLE" <<'PY'
import glob, json, statistics, sys

out, a_name, b_name, config_b, full = sys.argv[1:6]

def values(side):
    runs = []
    for path in sorted(glob.glob(f"{out}/{side}-*/arcade-milestone-1*.json")):
        runs.append(json.load(open(path)))
    return runs

# (label, getter, higher is better, decides the verdict)
metrics = []
loads = ["plain", "colored", "scroll", "fullscreen", "wide", "long-escapes", "marks"] if full == "1" else ["plain", "colored", "fullscreen"]
for load in loads:
    metrics.append((f"{load} MB/s", lambda r, l=load: r["throughput"]["arcade"][l]["mb_per_s"], True, True))
for key, label in (("arcade_idle", "key latency idle"), ("arcade_other_pane_flooding", "key latency flooding")):
    metrics.append((f"{label} p50 us", lambda r, k=key: r["latency"][k]["round_trip"]["p50_us"], False, True))
    metrics.append((f"{label} p99 us", lambda r, k=key: r["latency"][k]["round_trip"]["p99_us"], False, False))

a_runs, b_runs = values("A"), values("B")
rows, slower, overlapping, noisy = [], [], [], []
for label, get, higher_better, decides in metrics:
    try:
        a = [get(r) for r in a_runs]
        b = [get(r) for r in b_runs]
    except KeyError:
        continue
    if not a or not b:
        continue
    ma, mb = statistics.median(a), statistics.median(b)
    diff = (mb - ma) / ma * 100 if ma else 0.0
    worse = -diff if higher_better else diff
    rows.append((label, ma, mb, diff, (min(a), max(a)), (min(b), max(b)), decides))
    if decides and worse > 2.0:
        slower.append(label)
        if not (mb < min(a) or mb > max(a)):
            overlapping.append(label)

print(f"A: {a_name}")
print(f"B: {b_name}" + (f"   (with {config_b})" if config_b else ""))
print(f"{len(a_runs)} runs of A and {len(b_runs)} of B, interleaved; median and [lowest - highest]\n")
print(f"{'':26} {'median A':>10} {'median B':>10} {'B vs A':>8}   {'spread A':>17}   {'spread B':>17}")
for label, ma, mb, diff, sa, sb, decides in rows:
    wide = max((sa[1] - sa[0]), (sb[1] - sb[0])) / ma * 100 if ma else 0.0
    mark = "" if decides else "  (not judged)"
    if wide > 10.0:
        mark += "  (noisy: spread over 10%)"
        noisy.append(label)
    print(f"{label:26} {ma:10.1f} {mb:10.1f} {diff:+7.1f}%   [{sa[0]:7.1f} - {sa[1]:7.1f}]   [{sb[0]:7.1f} - {sb[1]:7.1f}]{mark}")
print()
if noisy:
    print("Noisy rows (" + ", ".join(noisy) + "): the spread of the runs is wider than 10 percent, so a difference there may be the machine. Run again when it is quiet.")
if slower:
    print("B is slower by more than 2 percent on: " + ", ".join(slower))
    if len(overlapping) == len(slower):
        print("(on each, B's median lies inside A's spread: the machine may be to blame; run it again)")
    sys.exit(1)
print("within spread")
PY
STATUS=$?
set -e
cat "$TABLE"
echo
echo "table: $TABLE"
echo "total: $(( $(date +%s) - START_ALL )) s"
rm -rf "$OUT"
exit "$STATUS"
