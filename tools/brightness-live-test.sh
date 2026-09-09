#!/usr/bin/env bash
set -euo pipefail

[[ ${EUID:-$(id -u)} -eq 0 ]] || {
  printf 'brightness-live-test: run with sudo\n' >&2
  exit 1
}

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
als_loaded=false

cleanup() {
  set +e
  if [[ $als_loaded == true ]] && grep -q '^aorus_als ' /proc/modules; then
    rmmod aorus_als
  fi
  make -C "$root/brightness/als" clean >/dev/null 2>&1
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# An earlier version registered a product-wide HID driver. hid-generic then
# relinquished every interface of the composite keyboard. Recover that state
# before doing anything else, and never rebind HID devices here again.
recovered=false
if grep -q '^aorus_brightness ' /proc/modules; then
  rmmod aorus_brightness
  recovered=true
fi
for candidate in /sys/bus/hid/devices/0003:1044:7A3A.*; do
  [[ -e $candidate ]] || continue
  if [[ ! -L $candidate/driver ]]; then
    printf '%s\n' "${candidate##*/}" > /sys/bus/hid/drivers/hid-generic/bind
    recovered=true
  fi
done
if [[ $recovered == true ]]; then
  printf 'Restored all GIGABYTE keyboard interfaces to hid-generic.\n'
  printf 'The unsafe HID test has been removed; no further key test was run.\n'
  exit 0
fi

make -C "$root/brightness/als" W=1 check >/dev/null

insmod "$root/brightness/als/aorus-als.ko"
als_loaded=true
iio=
for candidate in /sys/bus/iio/devices/iio:device*; do
  [[ $(cat "$candidate/name" 2>/dev/null || :) == aorus-ambient-light ]] || continue
  iio=$candidate/in_illuminance_input
  break
done
[[ -n $iio ]] || {
  printf 'brightness-live-test: ambient-light IIO device did not appear\n' >&2
  exit 1
}

printf '\nMove a hand over and away from the ambient-light sensor during the next 15 seconds.\n'
lux=
for _ in {1..30}; do
  if value=$(cat "$iio" 2>/dev/null); then
    lux=$value
    printf 'Ambient light: %s lux\n' "$lux"
  fi
  sleep 0.5
done
[[ -n $lux ]] || {
  printf 'brightness-live-test: no valid ambient-light WMI sample arrived\n' >&2
  exit 1
}

printf '\nTemporary ambient-light test passed.\n'
printf 'Cleanup will now unload the ambient-light test module.\n'
