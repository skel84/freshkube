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
        fds=$(lsof -p "$pid" 2>/dev/null | tail -n +2 | wc -l)
    fi
    printf '%s,%s,%s\n' "$((rss / 1024))" "$(echo $threads)" "$(echo $fds)"
}

soak_run() {
    local pid=$1 csv=$2 every=$3 started now line
    started=$(date +%s)
    echo "second,rss_mb,threads,fds" >"$csv"
    while kill -0 "$pid" 2>/dev/null; do
        if line=$(soak_sample "$pid"); then
            now=$(date +%s)
            echo "$((now - started)),$line" >>"$csv"
        fi
        sleep "$every"
    done
}
