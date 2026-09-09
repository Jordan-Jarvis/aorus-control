#!/usr/bin/env bash
set -Eeuo pipefail

die() {
    printf 'brightness-hid-capture: %s\n' "$*" >&2
    exit 2
}

[[ ${EUID:-$(id -u)} -eq 0 ]] || die 'run this diagnostic with sudo'
command -v python3 >/dev/null || die 'python3 is required'

output_dir=$(mktemp -d "${TMPDIR:-/tmp}/aorus-brightness-hid.XXXXXX")
reader_pid=
restore_owner() {
    if [[ -n $reader_pid ]]; then
        kill "$reader_pid" 2>/dev/null || true
        wait "$reader_pid" 2>/dev/null || true
    fi
    if [[ ${SUDO_UID:-} =~ ^[0-9]+$ && ${SUDO_GID:-} =~ ^[0-9]+$ ]]; then
        chown -R -- "$SUDO_UID:$SUDO_GID" "$output_dir" 2>/dev/null ||
            printf 'brightness-hid-capture: warning: output remains root-owned: %s\n' "$output_dir" >&2
    fi
}
trap restore_owner EXIT

hidraw_nodes=()
for device in /sys/class/hidraw/hidraw*; do
    [[ -r $device/device/uevent ]] || continue
    if grep -q '^HID_ID=0003:00001044:00007A3A$' "$device/device/uevent"; then
        node=/dev/${device##*/}
        [[ -r $node ]] || continue
        hidraw_nodes+=("$node")
        {
            cat "$device/device/uevent"
            printf 'SYSFS=%s\n' "$(readlink -f "$device/device")"
            printf 'REPORT_DESCRIPTOR='
            od -An -tx1 -v "$device/device/report_descriptor" | tr -d ' \n'
            printf '\n'
        } >"$output_dir/${device##*/}.txt"
    fi
done
((${#hidraw_nodes[@]} > 0)) || die 'no readable GIGABYTE 1044:7A3A hidraw device found'

printf 'Release all keys. Raw HID capture begins in three seconds.\n'
sleep 3
python3 -u - "$output_dir/reports.tsv" "${hidraw_nodes[@]}" <<'PY' &
import os
import selectors
import sys
import time

output, *paths = sys.argv[1:]
selector = selectors.DefaultSelector()
fds = []
try:
    for path in paths:
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
        fds.append(fd)
        selector.register(fd, selectors.EVENT_READ, path)
    with open(output, "w", buffering=1) as log:
        log.write("unix_ns\tdevice\treport_hex\n")
        while True:
            for key, _ in selector.select():
                try:
                    report = os.read(key.fd, 4096)
                except BlockingIOError:
                    continue
                if report:
                    log.write(f"{time.time_ns()}\t{key.data}\t{report.hex()}\n")
finally:
    for fd in fds:
        os.close(fd)
PY
reader_pid=$!

mark() {
    printf '%s\t%s\n' "$(date +%s%N)" "$1" >>"$output_dir/markers.tsv"
}

printf 'NOW press Fn+brightness-down once, then release it.\n'
mark brightness_down
sleep 5
printf 'NOW press Fn+brightness-up once, then release it.\n'
mark brightness_up
sleep 5
mark capture_end
kill "$reader_pid" 2>/dev/null || true
wait "$reader_pid" 2>/dev/null || true
reader_pid=

printf '\nMarkers:\n'
cat "$output_dir/markers.tsv"
printf '\nRaw reports:\n'
cat "$output_dir/reports.tsv"
printf '\nCapture complete: %s\n' "$output_dir"
