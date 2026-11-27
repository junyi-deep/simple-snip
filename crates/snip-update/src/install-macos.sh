pid=$1
target=$2
payload=$3
backup=$4
ready=$5
result=$6
reset_screen_capture=$7
reset_tool=$8
open_tool=$9

reset_permission() {
    # tccutil runs in the logged-in user's context; root has a wider scope.
    [ "$(/usr/bin/id -u)" -ne 0 ] || return 1
    "$reset_tool" reset ScreenCapture dev.simplesnip.desktop &
    reset_pid=$!
    count=0
    while kill -0 "$reset_pid" 2>/dev/null; do
        count=$((count + 1))
        if [ "$count" -ge 50 ]; then
            kill -KILL "$reset_pid" 2>/dev/null || :
            break
        fi
        sleep 0.1
    done
    wait "$reset_pid"
}

printf ready > "$ready" || exit 1
count=0
while kill -0 "$pid" 2>/dev/null; do
    count=$((count + 1))
    [ "$count" -lt 60 ] || exit 1
    sleep 1
done

if mv "$target" "$backup" && mv "$payload" "$target"; then
    set -- "$target"
    status=ok
    if [ "$reset_screen_capture" = yes ]; then
        if reset_permission; then
            status=ok-screen-capture-reset
            set -- "$@" --args --screen-capture-reset=done
        else
            status=ok-screen-capture-reset-failed
            set -- "$@" --args --screen-capture-reset=failed
        fi
    fi
    printf '%s' "$status" > "$result"
    if "$open_tool" "$@"; then
        exit 0
    fi
    mv "$target" "$payload.failed"
fi

[ ! -e "$backup" ] || mv "$backup" "$target"
printf failed > "$result"
"$open_tool" "$target"
