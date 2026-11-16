# Sampling a running process for a soak run. Source it (bash or zsh).
#
#   soak_sample <pid>      prints "rss_mb,threads,fds" for the process
#   soak_run <pid> <csv> <every-seconds>
#                          appends "second,rss_mb,threads,fds" to <csv> (with
#                          its header first) until the process ends
#
# Memory is the resident size from ps. Threads and open handles come from
# /proc on Linux, and from `ps -M` and `lsof` on macOS. Tokio tasks can't be
# counted from outside the process; threads and handles are what a leak of
# them shows in.

soak_sample() {
    local pid=$1 rss threads fds
    rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ')
    [ -n "$rss" ] || return 1
    if [ -d "/proc/$pid/task" ]; then
        threads=$(ls "/proc/$pid/task" | wc -l)
        fds=$(ls "/proc/$pid/fd" 2>/dev/null | wc -l)
    else
        threads=$(ps -M -p "$pid" 2>/dev/null | tail -n +2 | wc -l)
        # Numeric descriptors only: mapped libraries would lift the baseline
        # and hide a leaked socket.
        fds=$(lsof -n -P -a -p "$pid" -d 0-65535 2>/dev/null | tail -n +2 | wc -l)
    fi
    printf '%s,%s,%s\n' "$((rss / 1024))" "$(echo $threads)" "$(echo $fds)"
}

soak_run() {
    local pid=$1 csv=$2 every=$3 started now line waited
    started=$(date +%s)
    echo "second,rss_mb,threads,fds" >"$csv"
    while kill -0 "$pid" 2>/dev/null; do
        if line=$(soak_sample "$pid"); then
            now=$(date +%s)
            echo "$((now - started)),$line" >>"$csv"
        fi
        # In one-second steps, so a run that ends is noticed at once.
        waited=0
        while [ "$waited" -lt "$every" ] && kill -0 "$pid" 2>/dev/null; do
            sleep 1
            waited=$((waited + 1))
        done
    done
}

# soak_exec <csv> <every-seconds> <log> <command...>
#   Runs the command with its stderr in <log>, samples it into <csv> until it
#   ends, and returns its exit status (never stopping the caller's `set -e`
#   before the caller has shown the log). Ctrl-C or a terminated script ends
#   the command too: a background command ignores SIGINT in non-interactive
#   zsh, so it would otherwise run on, drawing, for hours. SOAK_RELEASE names
#   a command to run last (stress.sh gives the screen lock's release). The
#   machine is kept awake for the run when `caffeinate` exists.
soak_exec() {
    local csv=$1 every=$2 log=$3 rc=0
    shift 3
    "$@" 2>"$log" &
    SOAK_PID=$!
    trap 'kill $SOAK_PID 2>/dev/null || true; ${SOAK_RELEASE:-:}' EXIT INT TERM
    if command -v caffeinate >/dev/null; then caffeinate -dimsu -w $SOAK_PID & fi
    soak_run "$SOAK_PID" "$csv" "$every"
    wait $SOAK_PID || rc=$?
    return $rc
}

# soak_default_warmup <scenario> [args...]
#   Prints the seconds a soak run leaves out before judging growth: 600, or
#   longer when the scenario's own in-process generator is still filling its
#   replay buffer. `burst` and `summary-burst` keep the last 100,000 changes
#   (REPLAY_LIMIT in bin/stress/summary.rs, about 26 MB), which fills in
#   100,000 / rate seconds; the warm-up covers that and two minutes more.
soak_default_warmup() {
    local rate=""
    case ${1:-} in
        summary-burst | summary-410) rate=${2:-2000} ;;
        burst) rate=${3:-2000} ;;
    esac
    local warm=600
    if [ -n "$rate" ] && [ "$rate" -gt 0 ] 2>/dev/null; then
        local fill=$((100000 / rate + 120))
        [ "$fill" -gt "$warm" ] && warm=$fill
    fi
    echo "$warm"
}
