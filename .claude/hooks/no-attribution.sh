#!/usr/bin/env bash
# Refuses a shell command whose commit message, pull request or issue text
# carries a tool footer or a co-author trailer, including text in a body file
# the command names. Exit code 2 blocks the command and shows the reason.
set -euo pipefail
input="$(cat)"
command="$(printf '%s' "$input" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("tool_input",{}).get("command",""))')"
text="$command"
for file in $(printf '%s' "$command" | grep -oE '(--body-file|--file|-F|-m|--message)[= ]+@?[^ ;|&]+' | sed -E 's/^[^ =]+[= ]+@?//'); do
  [ -f "$file" ] && text="$text"$'\n'"$(cat "$file")"
done
if printf '%s' "$text" | grep -qiE 'generated (by|with) \[|co-authored-by:|cl.ude\.ai/code/session_|^[a-z]+-session: https://'; then
  echo "blocked: remove the tool footer or co-author trailer from this text and retry" >&2
  exit 2
fi
