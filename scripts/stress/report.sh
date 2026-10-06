# Judges a Monitoring stress run from its log. Sourced by stress.sh (zsh) and
# stress.test.sh (bash).
#
#   monitoring_verdict LOG KEYS   prints the verdict; fails when the run measured nothing
#
# A run whose summary has no monitoring.* span didn't draw the dashboard after
# the warm-up, and its process numbers would pass for a measurement. That
# fails, except when the keys only wait and the per-second lines show the
# dashboard drew during the warm-up: a page that stops drawing once its
# panels have answered is the result an idle run looks for.

monitoring_verdict() {
    local log=$1 keys=$2 last
    if grep -q '^summary monitoring\.' "$log"; then
        return 0
    fi
    last=$(sed -n 's/^perf \([0-9][0-9]*\) monitoring\..*/\1/p' "$log" | sort -n | tail -n 1)
    if [ -n "$last" ] && [ -n "$keys" ] && ! printf '%s\n' "$keys" | tr -s ' ' '\n' | grep -qv '^wait:[0-9][0-9]*$'; then
        echo "monitoring: 0 frames after warmup; the dashboard last drew in second $last"
        return 0
    fi
    if [ -n "$last" ]; then
        echo "No monitoring.* span after the warm-up: the dashboard last drew in second $last. See $log." >&2
    else
        echo "No monitoring.* span: the dashboard never drew. See $log." >&2
    fi
    return 1
}
