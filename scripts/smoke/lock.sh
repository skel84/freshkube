# One screen, one user at a time. Sourced by smoke.sh, browser.sh and
# stress.sh (bash or zsh). A smoke test brings its window to the front and
# types into it, and a stress run measures frame times, so two worktrees doing
# either at once spoil each other's captures, keys and numbers.
#
# The lock is a directory, created atomically. Its `owner` file names the
# worktree, the kind of use and the process to watch. Another worktree waits
# until the owner releases it, its process dies, or a smoke or browser owner
# has issued no command for FRESHKUBE_SCREEN_IDLE seconds (default 600).
#
#   screen_acquire KIND PID   wait for the screen, then hold it (re-entrant per worktree)
#   screen_owner_pid PID      watch another process, e.g. the app once it's up
#   screen_touch              note activity; fails if another worktree holds the screen
#   screen_release            let the next worktree in
#
# A locked screen spoils the same things: macOS shows the lock screen to
# captures, GPUI stops drawing a window it thinks is covered, and keystrokes go
# nowhere. Taking or using the screen therefore waits until it is unlocked.

SCREEN_LOCK=${FRESHKUBE_SCREEN_LOCK:-$HOME/.cache/freshkube/screen.lock}
SCREEN_IDLE=${FRESHKUBE_SCREEN_IDLE:-600}
SCREEN_WAIT=${FRESHKUBE_SCREEN_WAIT:-3600}

screen_locked() { ioreg -n Root -d1 | grep -q '"CGSSessionScreenIsLocked"=Yes'; }

# Wait for the user to unlock the screen, holding on to the lock meanwhile.
screen_wait_unlocked() {
  local waited=0
  screen_locked || return 0
  echo "screen: locked; waiting for the user to unlock it (captures and keys need an unlocked screen)" >&2
  while screen_locked; do
    if [ "$waited" -ge "$SCREEN_WAIT" ]; then
      echo "screen: still locked after ${SCREEN_WAIT}s" >&2
      return 1
    fi
    screen_mine && touch "$SCREEN_LOCK/owner"
    sleep 10
    waited=$((waited + 10))
  done
  echo "screen: unlocked" >&2
  # Windows draw again only once the session is back; give them a moment.
  sleep 2
}

screen_field() { sed -n "$1p" "$SCREEN_LOCK/owner" 2>/dev/null; }

screen_mine() { [ "$(screen_field 1)" = "$ROOT" ]; }

screen_write() { printf '%s\n%s\n%s\n%s\n' "$ROOT" "$1" "$2" "$(date '+%H:%M:%S')" >"$SCREEN_LOCK/owner"; }

# Stale: the owner's process is gone, or a smoke or browser owner went idle.
# A half-written lock (no owner file yet) counts as stale after a minute.
screen_stale() {
  local kind pid age
  if [ ! -f "$SCREEN_LOCK/owner" ]; then
    age=$(($(date +%s) - $(stat -f %m "$SCREEN_LOCK" 2>/dev/null || date +%s)))
    [ "$age" -gt 60 ]
    return
  fi
  kind=$(screen_field 2) pid=$(screen_field 3)
  kill -0 "$pid" 2>/dev/null || return 0
  [ "$kind" = stress ] && return 1
  age=$(($(date +%s) - $(stat -f %m "$SCREEN_LOCK/owner")))
  [ "$age" -gt "$SCREEN_IDLE" ]
}

screen_acquire() {
  local kind=$1 pid=$2 waited=0 told=""
  mkdir -p "$(dirname "$SCREEN_LOCK")"
  while :; do
    if mkdir "$SCREEN_LOCK" 2>/dev/null; then
      screen_write "$kind" "$pid"
      screen_wait_unlocked
      return
    fi
    if screen_mine; then
      screen_write "$kind" "$pid"
      screen_wait_unlocked
      return
    fi
    if screen_stale; then
      # An idle smoke app keeps running; stop it so its window leaves the screen.
      [ "$(screen_field 2)" = smoke ] && kill "$(screen_field 3)" 2>/dev/null
      echo "screen: taking over a stale lock from $(screen_field 1)" >&2
      rm -rf "$SCREEN_LOCK"
      continue
    fi
    if [ -z "$told" ]; then
      echo "screen: waiting for $(screen_field 1) ($(screen_field 2) since $(screen_field 4))" >&2
      told=1
    fi
    if [ "$waited" -ge "$SCREEN_WAIT" ]; then
      echo "screen: gave up after ${SCREEN_WAIT}s; $(screen_field 1) still holds it" >&2
      return 1
    fi
    sleep 5
    waited=$((waited + 5))
  done
}

screen_owner_pid() { screen_mine && screen_write "$(screen_field 2)" "$1"; }

screen_touch() {
  if screen_mine; then
    touch "$SCREEN_LOCK/owner"
    screen_wait_unlocked
    return
  fi
  if [ -d "$SCREEN_LOCK" ]; then
    echo "screen: $(screen_field 1) holds the screen ($(screen_field 2)); start again to wait for it" >&2
  else
    echo "screen: this worktree doesn't hold the screen; start again to take it" >&2
  fi
  return 1
}

screen_release() { screen_mine && rm -rf "$SCREEN_LOCK"; return 0; }
