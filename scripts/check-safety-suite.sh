#!/usr/bin/env bash
# Fails when a guarantee of the safety suite (docs/SAFETY.md) has no test.
# A suite test is a function named safety_gNN_<rule>, NN = 01..10.
set -euo pipefail
cd "$(dirname "$0")/.."
status=0
for n in 01 02 03 04 05 06 07 08 09 10; do
  if ! grep -rqE "fn safety_g${n}_[a-z0-9_]+" crates --include='*.rs'; then
    echo "safety suite: guarantee $n has no safety_g${n}_ test" >&2
    status=1
  fi
done
exit $status
