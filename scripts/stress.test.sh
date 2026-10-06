#!/usr/bin/env bash
# Checks scripts/stress/report.sh's verdicts on small Monitoring run logs.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0
. "$here/stress/report.sh"

# expect <exit> <description> <output substring> <keys> <log lines...>
expect() {
  local want="$1" what="$2" pattern="$3" keys="$4" got=0 out
  shift 4
  printf '%s\n' "$@" >"$work/run.log"
  out="$(monitoring_verdict "$work/run.log" "$keys" 2>&1)" || got=$?
  if [ "$got" != "$want" ] || { [ -n "$pattern" ] && ! grep -qF -- "$pattern" <<<"$out"; }; then
    echo "FAIL: $what (exit $got, wanted $want, output containing '$pattern')"
    sed 's/^/    /' <<<"$out"
    failures=$((failures + 1))
  else
    echo "ok: $what"
  fi
}

drew_early=(
  'perf 1 main.stall n=42 p50=18.42 p99=54.84 max=54.84 sum=839'
  'perf 1 monitoring.page_render n=41 p50=0.23 p99=0.64 max=0.64 sum=12'
  'perf 4 monitoring.page_render n=1 p50=0.16 p99=0.16 max=0.16 sum=0'
  'perf 9 main.stall n=179 p50=1.20 p99=4.50 max=12.78 sum=294'
  'summary main.stall n=4512 p50=1.20 p99=4.36 max=5.86 sum=7024'
  'summary process cpu n=25 p50=5.20 p99=5.36 max=5.36 sum=129 rss_end=219MB rss_max=223MB seconds=30.2'
)
never_drew=(
  'perf 1 main.stall n=42 p50=18.42 p99=54.84 max=54.84 sum=839'
  'summary main.stall n=4512 p50=1.20 p99=4.36 max=5.86 sum=7024'
  'summary process cpu n=25 p50=5.20 p99=5.36 max=5.36 sum=129 rss_end=219MB rss_max=223MB seconds=30.2'
)
measured=(
  'perf 7 monitoring.page_render n=30 p50=0.35 p99=1.87 max=6.65 sum=11'
  'summary monitoring.page_render n=717 p50=0.35 p99=1.87 max=6.65 sum=338'
  'summary process cpu n=25 p50=82.84 p99=85.86 max=85.86 sum=2068 rss_end=212MB rss_max=215MB seconds=31.3'
)

expect 0 'a measured run passes quietly' '' '' "${measured[@]}"
expect 0 'an idle run that drew during the warm-up reports 0 frames' \
  '0 frames after warmup; the dashboard last drew in second 4' 'wait:30000' "${drew_early[@]}"
expect 0 'several waits are still idle' '0 frames after warmup' 'wait:3000  wait:27000' "${drew_early[@]}"
expect 1 'an idle run that never drew fails' 'the dashboard never drew' 'wait:30000' "${never_drew[@]}"
expect 1 'the default keys that stop drawing fail' 'last drew in second 4' '' "${drew_early[@]}"
expect 1 'keys that hover and stop drawing fail' 'last drew in second 4' \
  'wait:6000 hover:0.3,0.5,0.6,0.5,2000' "${drew_early[@]}"
expect 1 'a run that never drew fails' 'the dashboard never drew' '' "${never_drew[@]}"

if [ "$failures" -ne 0 ]; then
  echo "$failures failed"
  exit 1
fi
