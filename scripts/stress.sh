#!/bin/zsh
# Runs one workload of the `stress` binary on a release build and prints its
# summary. The full per-second report is kept in target/stress/<label>.log.
#
#   scripts/stress.sh <label> <scenario> [args...]
#   scripts/stress.sh table-20k table 20000
#   scripts/stress.sh summary-20k summary
#   FRESHKUBE_STRESS_SECONDS=60 scripts/stress.sh logs-20k pod-logs 20000
#
# GPUI stops drawing a covered window or one on a locked screen, so run it
# with the screen unlocked and leave the window in front. It takes the shared
# screen lock (scripts/smoke/lock.sh) after building, so builds and other
# worktrees' smoke tests don't run during the measurement.
# FRESHKUBE_STRESS_SAMPLES=<csv> samples the process while it runs (soak.sh).
# FRESHKUBE_STRESS_BINARY can select a saved release binary for comparisons.
set -eu
label=$1; shift
root=${0:A:h:h}
out=$root/target/stress
mkdir -p $out
if ioreg -n Root -d1 -a | /usr/bin/grep -A1 CGSSessionScreenIsLocked | /usr/bin/grep -q '<true/>'; then
    echo "The screen is locked; the window wouldn't draw." >&2
    exit 1
fi
cd $root
if [[ -n ${FRESHKUBE_STRESS_BINARY:-} ]]; then
    stress_binary=$FRESHKUBE_STRESS_BINARY
else
    cargo build --release --quiet -p freshkube-desktop --features stress --bin stress
    build_target=${CARGO_TARGET_DIR:-$root/target}
    stress_binary=$build_target/release/stress
fi
# Wait for any other worktree's smoke test, browser check or stress run.
ROOT=$root
. $root/scripts/smoke/lock.sh
screen_acquire stress $$
trap screen_release EXIT
if [[ -n ${FRESHKUBE_STRESS_SAMPLES:-} ]]; then
    # A soak run (scripts/soak.sh): sample the process while it runs.
    . $root/scripts/soak/sample.sh
    "$stress_binary" "$@" 2> $out/$label.log &
    stress_pid=$!
    # A background command ignores SIGINT here, so Ctrl-C must end the app
    # itself, or it keeps drawing for hours after the lock is given up.
    # `|| true`: the app has usually ended already, and a failed kill must not
    # stop the trap before the lock is released (set -e).
    trap 'kill $stress_pid 2>/dev/null || true; screen_release' EXIT INT TERM
    # Keep the display and the machine awake for as long as the run lasts.
    if command -v caffeinate >/dev/null; then caffeinate -dimsu -w $stress_pid & fi
    soak_run $stress_pid $FRESHKUBE_STRESS_SAMPLES ${FRESHKUBE_STRESS_SAMPLE_EVERY:-30}
    # Not `wait` alone: under set -e a failing app would end the script before
    # its messages (a bad FRESHKUBE_STRESS_FAULTS) are shown below.
    run_rc=0
    wait $stress_pid || run_rc=$?
else
    "$stress_binary" "$@" 2> $out/$label.log
fi
# The fault counts are logged cumulatively as faults happen (a native quit
# doesn't return through main), so only the last of them is shown.
/usr/bin/grep -E '^(summary|stress:|stress server)' $out/$label.log | /usr/bin/grep -v '^stress server faults' || true
/usr/bin/grep '^stress server faults' $out/$label.log | tail -1 || true
if [[ ${run_rc:-0} != 0 ]]; then exit $run_rc; fi
if [[ $1 == monitoring ]]; then
    . $root/scripts/stress/report.sh
    monitoring_verdict $out/$label.log "${FRESHKUBE_STRESS_KEYS:-}"
fi
