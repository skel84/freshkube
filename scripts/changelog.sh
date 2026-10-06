#!/usr/bin/env bash
# Validate PR fragments or collect them into CHANGELOG.md for a release.
# changelog.py does the work; this checks that it looked at every fragment.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# Counted before Python runs, since collect removes them.
fragments="$(find "$root/changelog.d" -mindepth 2 -maxdepth 2 -name '*.md' | wc -l | tr -d ' ')"
# The script reads no input, so it never waits on the caller's stdin.
out="$(python3 "$root/scripts/changelog.py" "$root" "$@" </dev/null)"
printf '%s\n' "$out"
pattern='^changelog: (checked|collected) ([0-9]+) fragment'
if ! [[ "$out" =~ $pattern ]]; then
  echo "changelog: changelog.py reported nothing, so nothing was checked" >&2
  exit 1
fi
if [ "${BASH_REMATCH[2]}" != "$fragments" ]; then
  echo "changelog: ${BASH_REMATCH[1]} ${BASH_REMATCH[2]} fragment(s), but changelog.d holds $fragments" >&2
  exit 1
fi
