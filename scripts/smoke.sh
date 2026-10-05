#!/usr/bin/env bash
# Smoke-test the running app on macOS: start it on a page, capture its window,
# press keys and click, the way a person would. See AGENTS.md, "Smoke tests".
#
#   scripts/smoke.sh start [--page SLUG] [--theme light|dark] [--size WxH] [--release] [-- APP_ARGS...]
#   scripts/smoke.sh start --story SLUG [--theme light|dark] [--size WxH]   the workbench on a story
#   scripts/smoke.sh shot NAME [SETTLE_SECONDS]   capture the window to target/smoke/<worktree>/NAME.png
#   scripts/smoke.sh key 'keystroke "k" using command down'   any System Events key clause
#   scripts/smoke.sh click X Y                    click at points from the window's top-left
#   scripts/smoke.sh scroll X Y DY [DX]           scroll at X Y by DY points down, DX right
#   scripts/smoke.sh full NAME [X Y] [MAX]        NAME-0.png, NAME-1.png, … one per screenful,
#                                                 scrolling at X Y until the page stops moving
#   scripts/smoke.sh stop
#   scripts/smoke.sh pages [--wait SECONDS] [-- APP_ARGS...]   start on every page and capture each
#
# APP_ARGS default to --fixture; the workbench takes none (docs/WORKBENCH.md).
# For a live check, pass the context the user
# chose (`-- --config … --context … --kubeconfig …`); only look and navigate,
# never press Operations or maintenance actions.
# FRESHKUBE_SMOKE_BINARY selects a saved executable without rebuilding it.
#
# The terminal running this needs Screen Recording and Accessibility in
# System Settings → Privacy & Security, and the screen must be unlocked:
# GPUI stops drawing a covered window, and macOS drops keys sent to a window
# that isn't frontmost, so every command brings the app forward first.
#
# One worktree uses the screen at a time (scripts/smoke/lock.sh): start waits
# for any other worktree's smoke test, browser check or stress run, and stop
# lets the next one in. Always stop when done.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET=${CARGO_TARGET_DIR:-$ROOT/target}
OUT=$TARGET/smoke/$(basename "$ROOT")
HELPER=$TARGET/smoke/bin/window
PIDFILE=$OUT/pid
mkdir -p "$OUT" "$(dirname "$HELPER")"
. "$ROOT/scripts/smoke/lock.sh"

PAGES=(overview nodes health resources etcd system-services security lifecycle operations monitoring
  observability-applications observability-service-map observability-incidents
  observability-traces observability-profiling observability-deployments)

helper() {
  if [[ ! -x $HELPER || $ROOT/scripts/smoke/window.swift -nt $HELPER ]]; then
    swiftc -O -o "$HELPER" "$ROOT/scripts/smoke/window.swift"
  fi
  "$HELPER" "$@"
}

pid() {
  [[ -f $PIDFILE ]] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null || { echo "smoke: the app isn't running; use start" >&2; exit 1; }
  cat "$PIDFILE"
}

front() {
  screen_touch || exit 1
  osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $(pid)) to true" >/dev/null
  sleep 0.3
}

stop_app() {
  if [[ -f $PIDFILE ]]; then
    local p; p=$(cat "$PIDFILE")
    kill "$p" 2>/dev/null || true
    for _ in {1..50}; do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
    kill -9 "$p" 2>/dev/null || true
    rm -f "$PIDFILE"
  fi
}

stop() {
  stop_app
  screen_release
}

start() {
  local page="" story="" theme="" size="" profile=debug
  while [[ $# -gt 0 ]]; do
    case $1 in
      --page) page=$2; shift 2 ;;
      --story) story=$2; shift 2 ;;
      --theme) theme=$2; shift 2 ;;
      --size) size=$2; shift 2 ;;
      --release) profile=release; shift ;;
      --) shift; break ;;
      *) echo "smoke: unknown option $1" >&2; exit 2 ;;
    esac
  done
  local args=("$@") bin=freshkube
  if [[ -n $story ]]; then
    if [[ -n $page || ${#args[@]} -gt 0 ]]; then
      echo "smoke: --story takes no --page or APP_ARGS" >&2
      exit 2
    fi
    bin=freshkube-workbench
  elif [[ ${#args[@]} -eq 0 ]]; then
    args=(--fixture)
  fi
  local binary=${FRESHKUBE_SMOKE_BINARY:-$TARGET/$profile/$bin}
  if [[ -n ${FRESHKUBE_SMOKE_BINARY:-} ]]; then
    [[ $binary == /* ]] || binary=$PWD/$binary
    if [[ ! -f $binary || ! -x $binary ]]; then
      echo "smoke: FRESHKUBE_SMOKE_BINARY must name an executable file: $binary" >&2
      exit 1
    fi
  elif [[ $profile == release ]]; then
    cargo build -q --release --manifest-path "$ROOT/Cargo.toml" --bin "$bin"
  else
    cargo build -q --manifest-path "$ROOT/Cargo.toml" --bin "$bin"
  fi
  helper id 0 >/dev/null 2>&1 || true # build the helper before timing the window
  screen_acquire smoke $$
  stop_app
  # A new build asks Keychain for a remembered Coroot key; only the user can
  # answer, so call them with a sound on a live run.
  local stamp=$OUT/launched-$profile
  if [[ -z $story && ${args[0]} != --fixture && $binary -nt $stamp ]]; then
    afplay /System/Library/Sounds/Glass.aiff >/dev/null 2>&1 &
    echo "smoke: a new build; it may ask Keychain for the Coroot key, which the user answers" >&2
  fi
  touch "$stamp"
  # Bash 3.2 calls an empty array unbound, so the workbench's empty
  # arguments expand only when set.
  FRESHKUBE_PAGE=$page FRESHKUBE_STORY=$story FRESHKUBE_THEME=$theme FRESHKUBE_WINDOW_SIZE=$size \
    nohup "$binary" ${args[@]+"${args[@]}"} >"$OUT/app.log" 2>&1 &
  echo $! >"$PIDFILE"
  screen_owner_pid "$(cat "$PIDFILE")"
  for _ in {1..300}; do
    if helper id "$(pid)" >/dev/null 2>&1; then
      echo "smoke: $bin $(pid) is up${page:+ on $page}${story:+ on $story}"
      return
    fi
    sleep 0.1
  done
  echo "smoke: no window after 30 s; see $OUT/app.log" >&2
  stop
  exit 1
}

shot() {
  local name=${1:?name} settle=${2:-1}
  front
  sleep "$settle"
  screencapture -x -o -l "$(helper id "$(pid)")" "$OUT/$name.png"
  echo "$OUT/$name.png"
}

key() {
  front
  osascript -e "tell application \"System Events\" to ${1:?System Events key clause}"
}

click() {
  front
  local x y w h
  read -r x y w h < <(helper bounds "$(pid)")
  helper click $((x + ${1:?x})) $((y + ${2:?y}))
}

scroll() {
  front
  local x y w h
  read -r x y w h < <(helper bounds "$(pid)")
  helper scroll $((x + ${1:?x})) $((y + ${2:?y})) "${3:?dy}" ${4:+"$4"}
}

# The window between its header and status bar. The header's refresh ring and
# the status bar's clock and frame rate change on every capture, so `full`
# compares this part to tell when the page itself stopped moving.
content() {
  local x y w h
  read -r x y w h < <(helper bounds "$(pid)")
  screencapture -x -R "$((x + 80)),$((y + 80)),$((w - 80)),$((h - 120))" "$1"
}

# Every screenful of a scrolling page, top to bottom, at X Y (the middle of
# the window by default). Stops when the page no longer moves.
full() {
  local name=${1:?name} x=${2:-} y=${3:-} max=${4:-12} w h step
  read -r _ _ w h < <(helper bounds "$(pid)")
  x=${x:-$((w / 2))} y=${y:-$((h / 2))} step=$((h * 3 / 4))
  shot "$name-0" >/dev/null
  content "$OUT/full-prev.tmp.png"
  echo "$OUT/$name-0.png"
  for ((i = 1; i <= max; i++)); do
    scroll "$x" "$y" "$step"
    shot "$name-$i" 0.6 >/dev/null
    content "$OUT/full-next.tmp.png"
    if cmp -s "$OUT/full-next.tmp.png" "$OUT/full-prev.tmp.png"; then
      rm "$OUT/$name-$i.png"
      break
    fi
    mv "$OUT/full-next.tmp.png" "$OUT/full-prev.tmp.png"
    echo "$OUT/$name-$i.png"
  done
  rm -f "$OUT/full-prev.tmp.png" "$OUT/full-next.tmp.png"
}

pages() {
  local wait=4
  if [[ ${1:-} == --wait ]]; then wait=$2; shift 2; fi
  [[ ${1:-} == -- ]] && shift
  for page in "${PAGES[@]}"; do
    start --page "$page" -- "$@" >/dev/null
    shot "page-$page" "$wait"
  done
  stop
}

case ${1:-} in
  start | shot | key | click | scroll | full | stop | pages) cmd=$1; shift; "$cmd" "$@" ;;
  *) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
