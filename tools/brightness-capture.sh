#!/usr/bin/env bash
set -Eeuo pipefail
shopt -s nullglob

readonly DEFAULT_DURATION=20
duration=$DEFAULT_DURATION
output_dir=
generated_output=false
no_prompt=false
capture_pids=()
capture_labels=()
interrupted=false

cleanup_captures() {
    local pid
    for pid in "${capture_pids[@]}"; do
        kill -INT "$pid" 2>/dev/null || true
    done
}

handle_signal() {
    interrupted=true
    cleanup_captures
}

trap handle_signal INT TERM

restore_output_owner() {
    if [[ $generated_output == true && ${SUDO_UID:-} =~ ^[0-9]+$ && ${SUDO_GID:-} =~ ^[0-9]+$ ]]; then
        chown -R -- "$SUDO_UID:$SUDO_GID" "$output_dir" 2>/dev/null ||
            printf 'brightness-capture: warning: output remains root-owned: %s\n' "$output_dir" >&2
    fi
}

trap restore_output_owner EXIT

usage() {
    cat <<'EOF'
Usage: brightness-capture.sh [--duration SECONDS] [--output DIR] [--no-prompt]

Discover likely laptop input devices, record their events with every available
evtest/libinput backend, and save a read-only diagnostic bundle. The helper
never changes brightness, sysfs, udev, or system configuration.

By default it waits for Enter, then records for 20 seconds. During that window,
press Fn+brightness-down once and Fn+brightness-up once, and no other keys.
EOF
}

die() {
    printf 'brightness-capture: %s\n' "$*" >&2
    exit 2
}

have() {
    command -v "$1" >/dev/null 2>&1
}

read_sysfs() {
    local path=$1
    if [[ -r $path ]]; then
        tr '\n' ' ' <"$path"
        printf '\n'
    else
        printf '<unreadable>\n'
    fi
}

is_likely_device() {
    local name=${1,,}
    local phys=${2,,}
    local keycaps=${3:-}

    case "$name" in
        *mouse*|*'wireless radio'*)
            return 1
            ;;
    esac

    case "$name $phys" in
        *'video bus'*|*lnxvideo*)
            printf 'ACPI Video Bus\n'
            ;;
        *'at translated set 2'*|*i8042*)
            printf 'internal i8042 keyboard\n'
            ;;
        *gigabyte*|*aorus*)
            [[ -n ${keycaps//[[:space:]]/} && $keycaps != 0 ]] || return 1
            printf 'GIGABYTE laptop HID\n'
            ;;
        *)
            return 1
            ;;
    esac
}

start_capture() {
    local backend=$1
    local event_id=$2
    local node=$3
    local log_file=$4
    shift 4

    printf '%s\t%s\t%s\t%s\n' "$backend" "$event_id" "$node" "$*" >>"$output_dir/capture-commands.tsv"
    timeout --signal=INT --kill-after=2s "${duration}s" "$@" >"$log_file" 2>&1 &
    capture_pids+=("$!")
    capture_labels+=("$backend/$event_id")
}

while (($# > 0)); do
    case "$1" in
        --duration)
            (($# >= 2)) || die "--duration needs a positive integer"
            [[ $2 =~ ^[1-9][0-9]*$ ]] || die "duration must be a positive integer"
            duration=$2
            shift 2
            ;;
        --output)
            (($# >= 2)) || die "--output needs a directory"
            output_dir=$2
            shift 2
            ;;
        --no-prompt)
            no_prompt=true
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
done

have timeout || die "timeout is required (provided by coreutils)"

if [[ -z $output_dir ]]; then
    tmp_root=${TMPDIR:-/tmp}
    output_dir=$(mktemp -d "$tmp_root/aorus-brightness-capture.XXXXXX")
    generated_output=true
else
    mkdir -p -- "$output_dir"
fi

mkdir -p -- "$output_dir/devices" "$output_dir/events"
: >"$output_dir/capture-commands.tsv"
: >"$output_dir/capture-status.tsv"
printf 'backend\tevent_id\tnode\tcommand\n' >>"$output_dir/capture-commands.tsv"

{
    printf 'capture_started=%s\n' "$(date --iso-8601=seconds)"
    printf 'hostname=%s\n' "$(hostname 2>/dev/null || true)"
    printf 'kernel=%s\n' "$(uname -srvmo 2>/dev/null || true)"
    printf 'user=%s\n' "$(id -un 2>/dev/null || true)"
    printf '\n[dmi]\n'
    for field in sys_vendor product_name product_version board_vendor board_name board_version bios_vendor bios_version bios_date; do
        printf '%s=' "$field"
        read_sysfs "/sys/class/dmi/id/$field"
    done
    printf '\n[backlight]\n'
    for backlight in /sys/class/backlight/*; do
        [[ -d $backlight ]] || continue
        printf 'device=%s\n' "$backlight"
        for field in type max_brightness brightness actual_brightness bl_power scale; do
            printf '%s=' "$field"
            read_sysfs "$backlight/$field"
        done
    done
    printf '\n[modules]\n'
    if [[ -r /proc/modules ]]; then
        grep -Ei '^(aorus_laptop|system76_acpi|video|wmi|asus_wmi)[[:space:]]' /proc/modules 2>/dev/null || true
    fi
    printf '\n[acpi_and_wmi_paths]\n'
    find /sys/firmware/acpi /sys/bus/wmi/devices /sys/devices/platform -maxdepth 2 \
        \( -iname '*aorus*' -o -iname '*gigabyte*' -o -iname '*video*' -o -iname '*wmi*' -o -iname 'PNP0C14:*' \) \
        -print 2>/dev/null | sort || true
} >"$output_dir/system-state.txt"

printf 'event_id\tnode\tname\tphys\tsysfs\treason\n' >"$output_dir/input-devices.tsv"
likely_nodes=()
likely_ids=()
likely_reasons=()

for event_path in /sys/class/input/event*; do
    [[ -e $event_path ]] || continue
    event_id=${event_path##*/}
    node=/dev/input/$event_id
    name=$(read_sysfs "$event_path/device/name" | sed 's/[[:space:]]*$//')
    phys=$(read_sysfs "$event_path/device/phys" | sed 's/[[:space:]]*$//')
    keycaps=$(read_sysfs "$event_path/device/capabilities/key" | sed 's/[[:space:]]*$//')
    sysfs_path=$(readlink -f "$event_path/device" 2>/dev/null || printf '<unresolved>')
    reason=$(is_likely_device "$name" "$phys" "$keycaps" || true)
    printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$event_id" "$node" "$name" "$phys" "$sysfs_path" "${reason:-not-selected}" >>"$output_dir/input-devices.tsv"

    if [[ -n $reason && -e $node ]]; then
        likely_nodes+=("$node")
        likely_ids+=("$event_id")
        likely_reasons+=("$reason")
        {
            printf '[device]\npath=%s\nevent_id=%s\nname=%s\nphys=%s\nsysfs=%s\nreason=%s\n\n' \
                "$node" "$event_id" "$name" "$phys" "$sysfs_path" "$reason"
            printf '[permissions]\n'
            stat -c '%A %U:%G %n' "$node" 2>/dev/null || true
            printf '\n[udev]\n'
            if have udevadm; then
                udevadm info --query=property --name="$node" 2>&1 || true
            else
                printf 'udevadm=not-installed\n'
            fi
            printf '\n[capabilities]\n'
            for capability in key rel abs msc; do
                printf '%s=' "$capability"
                read_sysfs "$event_path/device/capabilities/$capability"
            done
        } >"$output_dir/devices/${event_id}.txt"
    fi
done

if ((${#likely_nodes[@]} == 0)); then
    die "no likely laptop input devices found; see $output_dir/input-devices.tsv"
fi

{
    printf 'Likely devices:\n'
    for i in "${!likely_nodes[@]}"; do
        printf '  %s: %s (%s)\n' "${likely_ids[$i]}" "${likely_nodes[$i]}" "${likely_reasons[$i]}"
    done
    printf '\nCapture directory: %s\n' "$output_dir"
    printf 'Backlight state and input metadata are in system-state.txt and input-devices.tsv.\n'
} | tee "$output_dir/README.txt"

if ! have evtest && ! have libinput; then
    printf '\nevtest and libinput are not installed; no event capture was started.\n' | tee -a "$output_dir/README.txt"
    printf 'Install both tools, then rerun this helper.\n'
    exit 3
fi

if [[ $no_prompt == false ]]; then
    [[ -t 0 ]] || die 'stdin is not a terminal; use --no-prompt for a scheduled capture'
    printf '\nPress Enter to start a %ss capture. During the window, press Fn+brightness-down once and Fn+brightness-up once; press no other keys.\n' "$duration"
    read -r
fi

for i in "${!likely_nodes[@]}"; do
    event_id=${likely_ids[$i]}
    node=${likely_nodes[$i]}
    if have evtest; then
        start_capture evtest "$event_id" "$node" "$output_dir/events/${event_id}.evtest.log" evtest "$node"
    fi
done

if have libinput; then
    libinput_help=$(libinput debug-events --help 2>&1 || true)
    libinput_args=(debug-events)
    if [[ $libinput_help == *--show-keycodes* ]]; then
        libinput_args+=(--show-keycodes)
    fi
    if [[ $libinput_help == *--device* ]]; then
        for i in "${!likely_nodes[@]}"; do
            event_id=${likely_ids[$i]}
            node=${likely_nodes[$i]}
            start_capture libinput "$event_id" "$node" "$output_dir/events/${event_id}.libinput.log" libinput "${libinput_args[@]}" --device "$node"
        done
    else
        printf 'libinput debug-events does not support --device; recording all libinput devices in all.libinput.log.\n' >>"$output_dir/README.txt"
        start_capture libinput all all "$output_dir/events/all.libinput.log" libinput "${libinput_args[@]}"
    fi
fi

for i in "${!capture_pids[@]}"; do
    status=0
    if wait "${capture_pids[$i]}"; then
        status=0
    else
        status=$?
    fi
    printf '%s\t%s\n' "${capture_labels[$i]}" "$status" >>"$output_dir/capture-status.tsv"
done

if [[ $interrupted == true ]]; then
    printf '\nCapture interrupted; review the partial logs in %s.\n' "$output_dir" | tee -a "$output_dir/README.txt"
    exit 130
fi

{
    printf '\nActual input-event search (absence is inconclusive unless both keys were pressed):\n'
    if grep -RHEn \
        -e '^[[:space:]]*Event: time .*(KEY_BRIGHTNESS(UP|DOWN)|MSC_SCAN|KEY_(UNKNOWN|RESERVED))' \
        -e '^[[:space:]]*event[0-9]+[[:space:]]+KEYBOARD_KEY[[:space:]].*(KEY_BRIGHTNESS(UP|DOWN)|MSC_SCAN|KEY_(UNKNOWN|RESERVED))' \
        "$output_dir/events" 2>/dev/null; then
        :
    else
        printf '  no matching lines found\n'
    fi
    printf '\nInterpretation:\n'
    printf '  standard KEY_BRIGHTNESSUP/DOWN -> inspect desktop/compositor handling; no hwdb mapping is needed.\n'
    printf '  MSC_SCAN or KEY_UNKNOWN/KEY_RESERVED without standard names -> preserve the exact captured scan value for a machine-specific hwdb investigation.\n'
    printf '  no matching lines after pressing both keys -> investigate ACPI/WMI/driver delivery.\n'
} | tee -a "$output_dir/README.txt"

printf '\nCapture complete: %s\n' "$output_dir"
