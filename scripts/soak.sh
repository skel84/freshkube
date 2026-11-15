#!/bin/zsh
# A long run of one stress scenario against the synthetic API with faults
# injected, sampling the app's memory, threads and open handles, and failing
# on steady growth.
#
#   scripts/soak.sh <label> <hours> <scenario> [args...]
#   scripts/soak.sh summary-burst-4h 4 summary-burst 500
#   FRESHKUBE_STRESS_FAULTS='refuse=0.05,gone=120' scripts/soak.sh quick 0.5 summary-burst 500
#
# Exit 0: no steady growth. 1: a series grew. 2: the run couldn't be judged
# (too short, the app died or the samples are missing).
#
# Knobs, all optional:
#   FRESHKUBE_STRESS_FAULTS  the fault plan; default drops, refuses, slows
#                            and expires watches (docs/PERFORMANCE.md)
#   SOAK_EVERY               seconds between samples; default 30
#   SOAK_WARMUP              seconds left out of the verdict; default 600
#
# Like scripts/stress.sh this needs the unlocked screen of a Mac; the run
# takes its lock. The samples are kept in target/soak/<label>.csv.
set -eu
label=$1 hours=$2; shift 2
root=${0:A:h:h}
out=$root/target/soak
mkdir -p $out
# zsh prints a fractional product as `360.`, which the stress binary can't read.
secs=$(( hours * 3600 ))
export FRESHKUBE_STRESS_SECONDS=${secs%.*}
export FRESHKUBE_STRESS_FAULTS=${FRESHKUBE_STRESS_FAULTS-refuse=0.01,drop=0.01,slow=0.02@1500,gone=300}
export FRESHKUBE_STRESS_SAMPLES=$out/$label.csv
export FRESHKUBE_STRESS_SAMPLE_EVERY=${SOAK_EVERY:-30}
warmup=${SOAK_WARMUP:-600}
export FRESHKUBE_STRESS_WARMUP=$warmup
rc=0
$root/scripts/stress.sh soak-$label "$@" || rc=$?
if [[ $rc != 0 ]]; then
    echo "soak: the run ended with status $rc" >&2
    exit 2
fi
cd $root
cargo run --quiet -p freshkube-probe --bin soak-verdict -- $out/$label.csv $warmup
