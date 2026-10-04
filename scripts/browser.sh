#!/usr/bin/env bash
# Look at a web page in Google Chrome the way smoke.sh looks at the app: to
# compare Freshkube with the tool it reads from, such as Coroot or Grafana.
#
#   scripts/browser.sh open URL [WxH]          a new Chrome window at the top-left (default 1792x1075)
#   scripts/browser.sh go URL [SETTLE]         load URL in that window
#   scripts/browser.sh shot NAME [SETTLE]      capture it to target/smoke/browser/NAME.png
#   scripts/browser.sh full NAME [X Y] [MAX]   NAME-0.png, NAME-1.png, … one per screenful,
#                                              scrolling at X Y until the page stops moving
#   scripts/browser.sh click X Y               click at points from the window's top-left
#   scripts/browser.sh scroll X Y DY           scroll at X Y by DY points; DY > 0 goes down
#   scripts/browser.sh key 'key code 121'      any System Events key clause
#   scripts/browser.sh url                     the address shown
#   scripts/browser.sh close
#
# Every command acts on Chrome's frontmost window, so leave the one `open`
# made in front. Sign-ins are the user's: when a page asks for one, stop and
# ask them to sign in in that window. Look and navigate only; keep
# credentials and secret values out of the captures you share.
#
# Needs the same Screen Recording and Accessibility permissions as smoke.sh.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
OUT=$TARGET/smoke/browser
HELPER=$TARGET/smoke/bin/window
mkdir -p "$OUT" "$(dirname "$HELPER")"

helper() {
  if [[ ! -x $HELPER || $ROOT/scripts/smoke/window.swift -nt $HELPER ]]; then
    swiftc -O -o "$HELPER" "$ROOT/scripts/smoke/window.swift"
  fi
  "$HELPER" "$@"
}

chrome() { osascript -e "tell application \"Google Chrome\" to $1"; }

pid() {
  pgrep -x "Google Chrome" | head -1 || { echo "browser: Chrome isn't running; use open" >&2; exit 1; }
}

front() {
  chrome activate >/dev/null
  sleep 0.3
}

open() {
  local url=${1:?url} size=${2:-1792x1075}
  local w=${size%x*} h=${size#*x}
  osascript >/dev/null <<OSA
tell application "Google Chrome"
  activate
  make new window
  set bounds of front window to {0, 25, $w, $((25 + h))}
  set URL of active tab of front window to "$url"
end tell
OSA
  sleep 3
  echo "browser: opened $url"
}

go() {
  chrome "set URL of active tab of front window to \"${1:?url}\"" >/dev/null
  sleep "${2:-6}"
}

shot() {
  local name=${1:?name} settle=${2:-1}
  front
  sleep "$settle"
  screencapture -x -o -l "$(helper id "$(pid)" front)" "$OUT/$name.png"
  echo "$OUT/$name.png"
}

click() {
  front
  local x y w h
  read -r x y w h < <(helper bounds "$(pid)" front)
  helper click $((x + ${1:?x})) $((y + ${2:?y}))
}

scroll() {
  front
  local x y w h
  read -r x y w h < <(helper bounds "$(pid)" front)
  helper scroll $((x + ${1:?x})) $((y + ${2:?y})) "${3:?dy}"
}

full() {
  local name=${1:?name} x=${2:-} y=${3:-} max=${4:-12} w h step
  read -r _ _ w h < <(helper bounds "$(pid)" front)
  x=${x:-$((w / 2))} y=${y:-$((h / 2))} step=$((h * 3 / 4))
  shot "$name-0" >/dev/null
  echo "$OUT/$name-0.png"
  for ((i = 1; i <= max; i++)); do
    scroll "$x" "$y" "$step"
    shot "$name-$i" 0.8 >/dev/null
    if cmp -s "$OUT/$name-$i.png" "$OUT/$name-$((i - 1)).png"; then
      rm "$OUT/$name-$i.png"
      break
    fi
    echo "$OUT/$name-$i.png"
  done
}

key() {
  front
  osascript -e "tell application \"System Events\" to ${1:?System Events key clause}"
}

url() { chrome 'get URL of active tab of front window'; }

close() { chrome 'close front window' >/dev/null; }

case ${1:-} in
  open | go | shot | full | click | scroll | key | url | close) cmd=$1; shift; "$cmd" "$@" ;;
  *) sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
