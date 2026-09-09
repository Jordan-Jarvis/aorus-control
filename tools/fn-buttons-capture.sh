#!/usr/bin/env bash
set -Eeuo pipefail

readonly HID_ID='0003:00001044:00007A3A'
readonly TAP_SECONDS=5
readonly HOLD_SECONDS=2
readonly HOLD_TAIL_SECONDS=3

readonly BUTTONS=(
    brightness-down
    brightness-up
    fan
    sleep
    wifi
    display
    square-x
    touchpad-lock
    ai
)

die() {
    printf 'fn-buttons-capture: %s\n' "$*" >&2
    exit 2
}

usage() {
    cat <<'USAGE'
Usage: sudo ./tools/fn-buttons-capture.sh --button NAME
       sudo ./tools/fn-buttons-capture.sh --all

Capture raw, read-only reports and evdev diagnostics for one or all AERO
Fn buttons. NAME is one of:
  brightness-down brightness-up fan sleep wifi display square-x
  touchpad-lock ai

Airplane mode is intentionally not included: Linux already handles that
button natively, so this diagnostic leaves it entirely to the OS.

The script never unbinds, grabs, writes to, or sends reports to a device.
USAGE
}

contains_button() {
    local wanted=$1
    local button
    for button in "${BUTTONS[@]}"; do
        [[ $button == "$wanted" ]] && return 0
    done
    return 1
}

button_arg=
capture_all=false
while (($# > 0)); do
    case $1 in
        --button)
            (($# >= 2)) || die '--button requires a name'
            [[ -z $button_arg && $capture_all == false ]] || die 'choose either --button or --all'
            button_arg=$2
            shift 2
            ;;
        --button=*)
            [[ -z $button_arg && $capture_all == false ]] || die 'choose either --button or --all'
            button_arg=${1#*=}
            shift
            ;;
        --all)
            [[ -z $button_arg && $capture_all == false ]] || die 'choose either --button or --all'
            capture_all=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            die "unknown argument: $1"
            ;;
    esac
done

if [[ $capture_all == true ]]; then
    selected_buttons=("${BUTTONS[@]}")
elif [[ -n $button_arg ]]; then
    contains_button "$button_arg" || die "unsupported button: $button_arg"
    selected_buttons=("$button_arg")
else
    usage >&2
    die 'specify --button NAME or --all'
fi

[[ ${EUID:-$(id -u)} -eq 0 ]] || die 'run this read-only diagnostic with sudo'
command -v python3 >/dev/null 2>&1 || die 'python3 is required'
command -v mktemp >/dev/null 2>&1 || die 'mktemp is required'

if [[ $capture_all == true ]]; then
    printf 'Airplane mode is intentionally left to Linux; it is not managed or captured.\n' >&2
fi

output_dir=$(mktemp -d "${TMPDIR:-/tmp}/aorus-fn-buttons.XXXXXX")
reader_pid=
inhibitor_pid=
evtest_pids=()
hidraw_nodes=()
hidraw_sysfs=()
event_nodes=()

owner_uid=${SUDO_UID:-$(id -u)}
owner_gid=${SUDO_GID:-$(id -g)}
if [[ ! $owner_uid =~ ^[0-9]+$ || ! $owner_gid =~ ^[0-9]+$ ]]; then
    owner_uid=$(id -u)
    owner_gid=$(id -g)
fi

cleanup() {
    local status=$?
    local pid

    trap - EXIT
    set +e

    if [[ -n ${reader_pid:-} ]]; then
        kill "$reader_pid" 2>/dev/null || true
        wait "$reader_pid" 2>/dev/null || true
    fi
    for pid in "${evtest_pids[@]:-}"; do
        [[ -n $pid ]] || continue
        kill "$pid" 2>/dev/null || true
    done
    for pid in "${evtest_pids[@]:-}"; do
        [[ -n $pid ]] || continue
        wait "$pid" 2>/dev/null || true
    done
    if [[ -n ${inhibitor_pid:-} ]]; then
        kill "$inhibitor_pid" 2>/dev/null || true
        wait "$inhibitor_pid" 2>/dev/null || true
    fi

    if [[ -d ${output_dir:-} ]]; then
        chown -R -- "$owner_uid:$owner_gid" "$output_dir" 2>/dev/null ||
            printf 'fn-buttons-capture: warning: output remains root-owned: %s\n' "$output_dir" >&2
        printf 'Capture complete: %s\n' "$output_dir" >&2
    fi
    return "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

uevent_has_target_hid_id() {
    local uevent=$1
    local line

    [[ -r $uevent ]] || return 1
    while IFS= read -r line; do
        [[ $line == "HID_ID=$HID_ID" ]] && return 0
    done <"$uevent" 2>/dev/null || true
    return 1
}

path_is_target_hid_descendant() {
    local current=$1
    local target

    [[ $current == /sys || $current == /sys/* ]] || return 1
    current=$(readlink -f -- "$current" 2>/dev/null) || return 1

    # The target HID interfaces are established by discover_hidraw() from
    # their exact HID_ID.  Compare resolved paths instead of walking every
    # sysfs ancestor: driver/module control entries expose unreadable uevent
    # files and are not input devices belonging to this laptop interface.
    for target in "${hidraw_sysfs[@]}"; do
        [[ $current == "$target" || $current == "$target/"* ]] && return 0
    done
    return 1
}

discover_hidraw() {
    local class_device node sysfs_path descriptor

    for class_device in /sys/class/hidraw/hidraw*; do
        uevent_has_target_hid_id "$class_device/device/uevent" || continue
        node=/dev/${class_device##*/}
        [[ -r $node ]] || continue
        sysfs_path=$(readlink -f -- "$class_device/device") || continue

        hidraw_nodes+=("$node")
        hidraw_sysfs+=("$sysfs_path")
        descriptor=unavailable
        if [[ -r $sysfs_path/report_descriptor ]]; then
            descriptor=$(od -An -tx1 -v "$sysfs_path/report_descriptor" 2>/dev/null | tr -d ' \n') ||
                descriptor=unavailable
        fi
        {
            printf 'node=%s\n' "$node"
            printf 'sysfs=%s\n' "$sysfs_path"
            printf 'driver=%s\n' "$(readlink -f -- "$sysfs_path/driver" 2>/dev/null || true)"
            printf 'report_descriptor=%s\n' "$descriptor"
            cat "$class_device/device/uevent"
        } >"$output_dir/${class_device##*/}.txt"
    done

    ((${#hidraw_nodes[@]} > 0)) || die 'no readable GIGABYTE 1044:7A3A hidraw interface found'
}

discover_events() {
    local class_device node sysfs_path

    for class_device in /sys/class/input/event*; do
        [[ -e $class_device/device ]] || continue
        # Input class entries are named eventN, but their device nodes live
        # below /dev/input.  Using /dev/eventN silently filtered every target
        # event device from the capture.
        node=/dev/input/${class_device##*/}
        [[ -r $node ]] || continue
        sysfs_path=$(readlink -f -- "$class_device/device") || continue

        if path_is_target_hid_descendant "$sysfs_path"; then
            event_nodes+=("$node")
        fi
    done

    {
        printf 'event_node\tsysfs\n'
        if ((${#event_nodes[@]} == 0)); then
            printf '# no readable event devices under the exact target HID interface found\n'
        else
            for node in "${event_nodes[@]}"; do
                printf '%s\t%s\n' "$node" "$(readlink -f -- "/sys/class/input/${node##*/}/device" 2>/dev/null || true)"
            done
        fi
    } >"$output_dir/event-devices.tsv"
}

read_file_or_unavailable() {
    local path=$1
    if [[ -r $path ]]; then
        cat -- "$path" 2>/dev/null || printf 'unreadable\n'
    else
        printf 'unavailable\n'
    fi
}

snapshot_command() {
    local label=$1
    shift
    local result status

    printf '[%s]\n' "$label"
    result=$("$@" 2>&1) && status=0 || status=$?
    printf '%s\n' "$result"
    if ((status != 0)); then
        printf 'command_exit=%s\n' "$status"
    fi
}

capture_state() {
    local phase=$1
    local state_file=$2
    local directory value found name lower_name

    {
        printf 'phase=%s\n' "$phase"
        printf 'timestamp_unix_ns=%s\n' "$(date +%s%N)"
        printf 'hid_id=%s\n' "$HID_ID"

        printf '\n[backlight]\n'
        found=0
        for directory in /sys/class/backlight/*; do
            [[ -d $directory ]] || continue
            found=1
            printf 'device=%s\n' "${directory##*/}"
            for value in actual_brightness brightness max_brightness type; do
                printf '%s=' "$value"
                read_file_or_unavailable "$directory/$value"
            done
        done
        ((found == 1)) || printf 'unavailable\n'

        printf '\n[system76-power-profile]\n'
        if command -v powerprofilesctl >/dev/null 2>&1; then
            snapshot_command powerprofilesctl-get powerprofilesctl get
            snapshot_command powerprofilesctl-list powerprofilesctl list
        else
            printf 'powerprofilesctl=unavailable\n'
        fi
        if command -v system76-power >/dev/null 2>&1; then
            snapshot_command system76-power-profile system76-power profile
        else
            printf 'system76-power=unavailable\n'
        fi

        printf '\n[networkmanager-radios]\n'
        if command -v nmcli >/dev/null 2>&1; then
            snapshot_command nmcli-radio-all nmcli radio all
        else
            printf 'nmcli=unavailable\n'
        fi

        printf '\n[touchpad]\n'
        found=0
        for directory in /sys/class/input/input*; do
            [[ -r $directory/name ]] || continue
            name=$(<"$directory/name")
            lower_name=${name,,}
            [[ $lower_name == *touchpad* ]] || continue
            found=1
            printf 'device=%s name=%s\n' "${directory##*/}" "$name"
            for value in enabled inhibited; do
                if [[ -r $directory/$value ]]; then
                    printf '%s=' "$value"
                    read_file_or_unavailable "$directory/$value"
                fi
                if [[ -r $directory/device/$value ]]; then
                    printf 'device/%s=' "$value"
                    read_file_or_unavailable "$directory/device/$value"
                fi
            done
        done
        if ((found == 0)); then
            printf 'sysfs_touchpad_state=unavailable\n'
        fi
        if command -v gsettings >/dev/null 2>&1; then
            if [[ -n ${SUDO_USER:-} && $SUDO_USER != root && -d /run/user/$owner_uid && -n $(command -v runuser || true) ]]; then
                snapshot_command gsettings-touchpad-send-events runuser -u "$SUDO_USER" -- env \
                    XDG_RUNTIME_DIR="/run/user/$owner_uid" \
                    DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$owner_uid/bus" \
                    gsettings get org.gnome.desktop.peripherals.touchpad send-events
            else
                snapshot_command gsettings-touchpad-send-events gsettings get \
                    org.gnome.desktop.peripherals.touchpad send-events
            fi
        else
            printf 'gsettings=unavailable\n'
        fi

        printf '\n[display]\n'
        found=0
        for directory in /sys/class/drm/card*-*; do
            [[ -d $directory ]] || continue
            found=1
            printf 'connector=%s status=' "${directory##*/}"
            read_file_or_unavailable "$directory/status"
            for value in enabled dpms modes; do
                [[ -r $directory/$value ]] || continue
                printf '%s=' "$value"
                read_file_or_unavailable "$directory/$value"
            done
        done
        ((found == 1)) || printf 'sysfs_drm=unavailable\n'
        if command -v xrandr >/dev/null 2>&1 && [[ -n ${DISPLAY:-} ]]; then
            snapshot_command xrandr-query xrandr --query
        fi
        if command -v wlr-randr >/dev/null 2>&1 && [[ -n ${WAYLAND_DISPLAY:-} ]]; then
            snapshot_command wlr-randr-query wlr-randr
        fi
        if command -v kscreen-doctor >/dev/null 2>&1 && [[ -n ${DISPLAY:-}${WAYLAND_DISPLAY:-} ]]; then
            snapshot_command kscreen-doctor-output kscreen-doctor -o
        fi

        printf '\n[hid-generic-bindings]\n'
        found=0
        if [[ -d /sys/bus/hid/drivers/hid-generic ]]; then
            for directory in /sys/bus/hid/drivers/hid-generic/*; do
                [[ -e $directory || -L $directory ]] || continue
                if [[ -d $directory ]] &&
                    path_is_target_hid_descendant "$(readlink -f -- "$directory" 2>/dev/null || true)"; then
                    found=1
                    printf '%s -> %s\n' "${directory##*/}" "$(readlink -f -- "$directory" 2>/dev/null || true)"
                fi
            done
        fi
        for ((value = 0; value < ${#hidraw_sysfs[@]}; value++)); do
            directory=${hidraw_sysfs[value]}
            found=1
            printf 'hidraw=%s driver=%s\n' "${hidraw_nodes[value]}" \
                "$(readlink -f -- "$directory/driver" 2>/dev/null || true)"
        done
        ((found == 1)) || printf 'unavailable\n'
    } >"$state_file"
}

start_evtest_logging() {
    local node log_file

    if ! command -v evtest >/dev/null 2>&1; then
        printf 'fn-buttons-capture: warning: evtest is unavailable; raw HID capture will continue\n' >&2
        return 0
    fi
    for node in "${event_nodes[@]}"; do
        log_file="$output_dir/${node##*/}.evtest.log"
        evtest "$node" >"$log_file" 2>&1 < /dev/null &
        evtest_pids+=("$!")
    done
}

start_raw_reader() {
    python3 -u - "$output_dir/reports.tsv" "${hidraw_nodes[@]}" >"$output_dir/raw-reader.log" 2>&1 <<'PY' &
import os
import selectors
import sys
import time

output, *paths = sys.argv[1:]
selector = selectors.DefaultSelector()
fds = []
try:
    for path in paths:
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_CLOEXEC", 0))
        except OSError as error:
            print(f"{path}: {error}", file=sys.stderr, flush=True)
            continue
        fds.append(fd)
        selector.register(fd, selectors.EVENT_READ, path)

    with open(output, "w", buffering=1) as log:
        log.write("unix_ns\tdevice\treport_hex\n")
        while True:
            for key, _ in selector.select(timeout=0.5):
                try:
                    report = os.read(key.fd, 4096)
                except BlockingIOError:
                    continue
                except OSError as error:
                    print(f"{key.data}: {error}", file=sys.stderr, flush=True)
                    continue
                if report:
                    log.write(f"{time.time_ns()}\t{key.data}\t{report.hex()}\n")
finally:
    for fd in fds:
        os.close(fd)
PY
    reader_pid=$!
}

start_sleep_inhibitor() {
    local seconds=$1
    if ! command -v systemd-inhibit >/dev/null 2>&1; then
        printf 'fn-buttons-capture: warning: systemd-inhibit is unavailable for sleep capture\n' >&2
        return 0
    fi
    systemd-inhibit --what=sleep --mode=block \
        --why='AORUS Fn-button sleep capture' sleep "$seconds" \
        >"$output_dir/sleep-inhibitor.log" 2>&1 &
    inhibitor_pid=$!
    sleep 0.2
    if ! kill -0 "$inhibitor_pid" 2>/dev/null; then
        printf 'fn-buttons-capture: warning: logind sleep inhibitor did not stay active\n' >&2
        wait "$inhibitor_pid" 2>/dev/null || true
        inhibitor_pid=
    fi
}

mark() {
    printf '%s\t%s\t%s\n' "$(date +%s%N)" "$1" "$2" >>"$output_dir/markers.tsv"
}

capture_button() {
    local button=$1

    printf '\n[%s] TAP: press once and release during the next %ss.\n' "$button" "$TAP_SECONDS"
    mark "$button" tap_start
    sleep "$TAP_SECONDS"
    mark "$button" tap_end

    printf '[%s] HOLD: press and hold for two seconds, then release. Window starts now.\n' "$button"
    mark "$button" hold_start
    sleep "$HOLD_SECONDS"
    mark "$button" hold_2s
    sleep "$HOLD_TAIL_SECONDS"
    mark "$button" hold_end
}

discover_hidraw
discover_events

{
    printf 'hid_id=%s\n' "$HID_ID"
    printf 'hidraw_node\tsysfs\n'
    for ((value = 0; value < ${#hidraw_nodes[@]}; value++)); do
        printf '%s\t%s\n' "${hidraw_nodes[value]}" "${hidraw_sysfs[value]}"
    done
} >"$output_dir/targets.tsv"
printf 'Release all keys. Read-only capture begins in three seconds.\n'
sleep 3

printf 'unix_ns\tbutton\tmarker\n' >"$output_dir/markers.tsv"
capture_state before "$output_dir/state-before.txt"
start_evtest_logging
start_raw_reader

printf '\nLikely event devices: %s\n' "${event_nodes[*]:-none}"
printf 'Raw reports are being recorded in %s/reports.tsv\n' "$output_dir"

for button in "${selected_buttons[@]}"; do
    if [[ $button == sleep ]]; then
        # A sleep key can suspend the laptop before its raw report is saved.
        # This inhibitor only blocks sleep for this diagnostic window.
        start_sleep_inhibitor "$((TAP_SECONDS + HOLD_SECONDS + HOLD_TAIL_SECONDS + 2))"
    fi
    capture_button "$button"
    if [[ -n $inhibitor_pid ]]; then
        kill "$inhibitor_pid" 2>/dev/null || true
        wait "$inhibitor_pid" 2>/dev/null || true
        inhibitor_pid=
    fi
done

capture_state after "$output_dir/state-after.txt"

if [[ -s $output_dir/markers.tsv ]]; then
    printf '\nMarkers:\n'
    cat "$output_dir/markers.tsv"
fi
printf '\nRaw reports:\n'
cat "$output_dir/reports.tsv"
