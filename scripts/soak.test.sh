#!/usr/bin/env bash
# Checks the soak run's sampler and verdict without a GUI: samples a plain
# process, then judges small CSVs.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$here")"
work="$(mktemp -d)"
trap 'kill "${sleeper:-0}" 2>/dev/null || true; rm -rf "$work"' EXIT
failures=0
. "$here/soak/sample.sh"

check() { # <description> <condition-exit-status>
  if [ "$2" = 0 ]; then echo "ok: $1"; else echo "FAIL: $1"; failures=$((failures + 1)); fi
}

# The sampler on a live process, and on one that is gone.
sleep 30 &
sleeper=$!
line="$(soak_sample "$sleeper")"
[[ "$line" =~ ^[0-9]+,[0-9]+,[0-9]+$ ]]; check "a live process gives rss_mb,threads,fds ($line)" $?
kill "$sleeper"; wait "$sleeper" 2>/dev/null || true
soak_sample "$sleeper" >/dev/null 2>&1 && status=0 || status=$?
[ "$status" != 0 ]; check "an ended process gives no sample" $?

# soak_run stops when its process ends, with a header and one line per sample.
sleep 3 &
sleeper=$!
soak_run "$sleeper" "$work/run.csv" 1
[ "$(head -1 "$work/run.csv")" = "second,rss_mb,threads,fds" ]; check "the CSV starts with its header" $?
[ "$(wc -l <"$work/run.csv")" -ge 2 ]; check "the CSV has samples" $?

# zsh runs soak.sh and stress.sh, where these names are read-only: assigning
# one stops the script. This test runs under bash, which would not notice.
! grep -nE '^[[:space:]]*(status|path|argv|pipestatus|options|commands)=' "$here/soak.sh" "$here/stress.sh"; check "no zsh read-only names are assigned" $?

# soak_run notices its process ending within a second, not after its interval.
sleep 2 &
sleeper=$!
begin=$SECONDS
soak_run "$sleeper" "$work/quick.csv" 30
[ $((SECONDS - begin)) -le 5 ]; check "a run that ends is noticed at once ($((SECONDS - begin)) s)" $?

cargo build --quiet --manifest-path "$root/Cargo.toml" -p freshkube-probe --bin soak-verdict
verdict="$root/target/debug/soak-verdict"

# series <file> <rss-start> <rss-step>: 60 samples a minute apart.
series() {
  { echo "second,rss_mb,threads,fds"
    for i in $(seq 0 59); do echo "$((i * 60)),$(( $2 + i * $3 )),40,60"; done; } >"$1"
}
expect() { # <description> <wanted exit> <file> <warmup>
  local got=0
  "$verdict" "$3" "$4" >"$work/out" 2>&1 || got=$?
  if [ "$got" = "$2" ]; then echo "ok: $1"; else
    echo "FAIL: $1 (exit $got, wanted $2)"; sed 's/^/    /' "$work/out"; failures=$((failures + 1)); fi
}
series "$work/flat.csv" 300 0
series "$work/leak.csv" 300 3
expect "a flat run passes" 0 "$work/flat.csv" 300
expect "a leaking run fails" 1 "$work/leak.csv" 300
expect "a run too short to judge is not a pass" 2 "$work/flat.csv" 3000
expect "a missing file is not a pass" 2 "$work/none.csv" 300
printf 'second,rss_mb,threads,fds\n1,2,x,4\n' >"$work/bad.csv"
expect "a bad line is not a pass" 2 "$work/bad.csv" 0

[ "$failures" = 0 ]
