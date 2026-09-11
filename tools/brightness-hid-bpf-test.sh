#!/usr/bin/env bash
set -euo pipefail

die() {
  printf 'brightness-hid-bpf-test: %s\n' "$*" >&2
  exit 2
}

[[ ${EUID:-$(id -u)} -eq 0 ]] || die 'run this guarded test with sudo'
[[ ${1:-} == --confirm-external-keyboard ]] ||
  die 'connect an external keyboard, then pass --confirm-external-keyboard'
command -v evtest >/dev/null || die 'evtest is required'

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if ! command -v udev-hid-bpf >/dev/null; then
  loader_dir="$root/target/udev-hid-bpf/target/release"
  [[ -x $loader_dir/udev-hid-bpf ]] || die 'build udev-hid-bpf with tools/brightness-hid-bpf-loader-build.sh'
  export PATH="$loader_dir:$PATH"
fi
helper="$root/packaging/aorus-brightness-hid-bpf"
runtime_helper=
runtime_loader=
object="$root/target/aorus-brightness.bpf.o"
installed_object=/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o
target=
watchdog_unit=

restore() {
  set +e
  recovery_helper=$helper
  [[ -z $runtime_helper || ! -x $runtime_helper ]] || recovery_helper=$runtime_helper
  [[ -z $target ]] || "$recovery_helper" recover "$target"
}

cleanup() {
  restore
  if [[ -n $watchdog_unit ]]; then
    systemctl stop "$watchdog_unit.timer" "$watchdog_unit.service" >/dev/null 2>&1 || true
    systemctl reset-failed "$watchdog_unit.service" >/dev/null 2>&1 || true
  fi
  [[ -z $runtime_helper ]] || rm -f -- "$runtime_helper"
  [[ -z $runtime_loader ]] || rm -f -- "$runtime_loader"
  [[ -z $runtime_helper ]] || rmdir -- "${runtime_helper%/*}" 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

for candidate in /sys/bus/hid/devices/0003:1044:7A3A.*; do
  [[ -e $candidate ]] || continue
  [[ $(cat "$candidate/../bInterfaceNumber" 2>/dev/null || true) == 02 ]] || continue
  target=$candidate
  break
done
[[ -n $target ]] || die 'GIGABYTE HID interface 2 was not found'

for candidate in /sys/bus/hid/devices/0003:1044:7A3A.*; do
  [[ -e $candidate ]] || continue
  [[ $(basename "$(readlink -f "$candidate/driver")") == hid-generic ]] ||
    die "${candidate##*/} is not already bound to hid-generic; no test was started"
done

[[ -f $object ]] || "$root/tools/brightness-hid-bpf-build.sh" "$object"
install -D -m 0644 "$object" "$installed_object"

# A transient systemd timer rolls back even if this shell, sudo, or the
# terminal dies during attachment.
watchdog_unit="aorus-brightness-recovery-$$"
# /run is commonly mounted noexec, so the copied helper and loader must live
# on an executable filesystem for both this test and the systemd rollback.
runtime_dir=$(mktemp -d "/var/tmp/$watchdog_unit.XXXXXX")
runtime_helper="$runtime_dir/aorus-brightness-hid-bpf"
runtime_loader="$runtime_dir/udev-hid-bpf"
install -D -m 0755 "$helper" "$runtime_helper"
install -m 0755 "$(command -v udev-hid-bpf)" "$runtime_loader"
export PATH="$runtime_dir:$PATH"
systemd-run --quiet --unit "$watchdog_unit" --on-active=45s \
  --timer-property=AccuracySec=1s --setenv=PATH="$PATH" \
  "$runtime_helper" recover "$target"

"$runtime_helper" test-add "$target"
sleep 2

for candidate in /sys/bus/hid/devices/0003:1044:7A3A.*; do
  [[ -e $candidate ]] || continue
  [[ $(basename "$(readlink -f "$candidate/driver")") == hid-generic ]] ||
    die "${candidate##*/} is not bound to hid-generic; rollback has started"
done

mapfile -t events < <(python3 - "$target" <<'PY'
import pathlib
import sys

target = pathlib.Path(sys.argv[1])
word_bits = 64
required = (224, 225)  # KEY_BRIGHTNESSDOWN, KEY_BRIGHTNESSUP
for path in target.glob("input/input*/event*"):
    words = (path.parent / "capabilities/key").read_text().split()
    values = [int(word, 16) for word in reversed(words)]
    if all(code // word_bits < len(values) and
           values[code // word_bits] & (1 << (code % word_bits))
           for code in required):
        print(f"/dev/input/{path.name}")
PY
)
(( ${#events[@]} > 0 )) || die 'the native brightness identity evdev device did not appear'

printf '\nNative HID-BPF is attached to %s; all keyboard interfaces remain on hid-generic.\n' "$target"
printf 'During the next 20 seconds: tap brightness-down, tap brightness-up, hold brightness-down for two seconds, then hold brightness-up for two seconds.\n\n'
log=$(mktemp -d)
pids=()
for event in "${events[@]}"; do
  timeout 20 evtest "$event" >"$log/${event##*/}.log" 2>&1 &
  pids+=("$!")
done
set +e
for pid in "${pids[@]}"; do
  wait "$pid"
done
set -e

down=$(grep -h 'KEY_BRIGHTNESSDOWN.*value 1' "$log"/*.log | wc -l || true)
up=$(grep -h 'KEY_BRIGHTNESSUP.*value 1' "$log"/*.log | wc -l || true)
printf 'Native brightness events: down=%s up=%s\n' "$down" "$up"
((down >= 2 && up >= 2)) || die "native brightness press/hold validation failed; event log: $log"

printf 'Guarded native brightness test passed. HID-BPF will now be detached.\n'
printf 'After installing AORUS Control, enable the production path in the app or with: aorusctl fn enable\n'
