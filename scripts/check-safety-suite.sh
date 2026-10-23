#!/usr/bin/env bash
# The safety suite (docs/SAFETY.md) is what the compiler builds: the tests
# `cargo test --workspace -- --list` finds named safety_gNN_… must be exactly
# the tests docs/SAFETY.md names, every guarantee that doc defines must have
# one, an empty list fails, and none may be #[ignore]d. A commented-out or
# cfg-gated function is not listed, so it cannot satisfy this.
set -euo pipefail
cd "$(dirname "$0")/.."

names='safety_g[0-9]{2}_[a-z0-9_]+'
listed=$(cargo test --workspace --locked safety_g -- --list 2>/dev/null |
  sed -nE "s/^(.*::)?($names): test\$/\2/p" | sort -u)
ignored=$(cargo test --workspace --locked safety_g -- --list --ignored 2>/dev/null |
  sed -nE "s/^(.*::)?($names): test\$/\2/p" | sort -u)
documented=$(grep -oE "\`$names\`" docs/SAFETY.md | tr -d '`' | sort -u)
guarantees=$(sed -nE 's/^### ([0-9]+)\. .*/\1/p' docs/SAFETY.md | xargs -r printf '%02d\n' | sort -u)

status=0
if [ -z "$listed" ]; then
  echo "safety suite: cargo lists no safety_g test" >&2
  exit 1
fi
if [ -z "$guarantees" ]; then
  echo "safety suite: docs/SAFETY.md defines no guarantee (### N. headings)" >&2
  exit 1
fi
for n in $guarantees; do
  if ! grep -q "^safety_g${n}_" <<<"$listed"; then
    echo "safety suite: guarantee $n has no listed safety_g${n}_ test" >&2
    status=1
  fi
done
if [ -n "$ignored" ]; then
  echo "safety suite: ignored tests are not allowed:" >&2
  echo "$ignored" >&2
  status=1
fi
extra=$(comm -23 <(echo "$listed") <(echo "$documented"))
missing=$(comm -13 <(echo "$listed") <(echo "$documented"))
if [ -n "$extra" ]; then
  echo "safety suite: tests not named in docs/SAFETY.md:" >&2
  echo "$extra" >&2
  status=1
fi
if [ -n "$missing" ]; then
  echo "safety suite: docs/SAFETY.md names tests that are not compiled:" >&2
  echo "$missing" >&2
  status=1
fi
exit $status
