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
# with the screen unlocked and leave the window in front.
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
"$stress_binary" "$@" 2> $out/$label.log
/usr/bin/grep -E '^(summary|stress server)' $out/$label.log || true
