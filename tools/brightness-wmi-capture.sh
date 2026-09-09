#!/usr/bin/env bash
set -Eeuo pipefail

die() {
    printf 'brightness-wmi-capture: %s\n' "$*" >&2
    exit 2
}

[[ ${EUID:-$(id -u)} -eq 0 ]] || die 'run this diagnostic with sudo'
[[ -t 0 ]] || die 'stdin must be a terminal'
command -v make >/dev/null || die 'make is required'
command -v insmod >/dev/null || die 'insmod is required'
command -v rmmod >/dev/null || die 'rmmod is required'
command -v journalctl >/dev/null || die 'journalctl is required'

root_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
source_file=$root_dir/brightness/aorus-hotkey-trace.c
kernel_build=/lib/modules/$(uname -r)/build
[[ -r $source_file ]] || die "missing $source_file"
[[ -d $kernel_build ]] || die "missing kernel headers at $kernel_build"
[[ ! -d /sys/module/aorus_hotkey_trace ]] || die 'aorus_hotkey_trace is already loaded'

output_dir=$(mktemp -d "${TMPDIR:-/tmp}/aorus-brightness-wmi.XXXXXX")
build_dir=$output_dir/build
mkdir -p "$build_dir"
cp -- "$source_file" "$build_dir/aorus-hotkey-trace.c"
cat >"$build_dir/Makefile" <<'EOF'
obj-m += aorus-hotkey-trace.o
KDIR ?= /lib/modules/$(shell uname -r)/build
all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules
EOF

loaded=false
restore() {
    if [[ $loaded == true ]]; then
        rmmod aorus_hotkey_trace 2>/dev/null || true
    fi
    if [[ ${SUDO_UID:-} =~ ^[0-9]+$ && ${SUDO_GID:-} =~ ^[0-9]+$ ]]; then
        chown -R -- "$SUDO_UID:$SUDO_GID" "$output_dir" 2>/dev/null ||
            printf 'brightness-wmi-capture: warning: output remains root-owned: %s\n' "$output_dir" >&2
    fi
}
trap restore EXIT

make -s -C "$build_dir" >"$output_dir/build.log" 2>&1 || {
    cat "$output_dir/build.log" >&2
    die 'temporary tracer module failed to build'
}

since=$(date --iso-8601=seconds)
insmod "$build_dir/aorus-hotkey-trace.ko"
loaded=true

printf 'Temporary read-only WMI tracer loaded.\n'
printf 'Press Fn+brightness-down once, then Fn+brightness-up once, then press Enter.\n'
read -r

journalctl -k --since "$since" --no-pager >"$output_dir/kernel.log"
grep -E 'aorus_hotkey_trace: (capturing|WMI event)' "$output_dir/kernel.log" |
    tee "$output_dir/events.txt" || true

if ! grep -q 'aorus_hotkey_trace: WMI event' "$output_dir/events.txt"; then
    printf 'No WMI event reached GUID %s.\n' 'ABBC0F72-8EA1-11D1-00A0-C90629100000'
fi
printf 'Capture complete: %s\n' "$output_dir"
