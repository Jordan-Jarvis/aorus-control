#!/usr/bin/env bash
set -euo pipefail

die() { printf 'install-als: %s\n' "$*" >&2; exit 1; }
[[ $EUID == 0 ]] || die 'run with sudo; this script never invokes sudo'
[[ $(cat /sys/class/dmi/id/sys_vendor) == GIGABYTE &&
   $(cat /sys/class/dmi/id/product_name) == 'AERO 16 YE5' &&
   $(cat /sys/class/dmi/id/product_version) == P86VE ]] || die 'unsupported laptop'
[[ -d /sys/bus/wmi/devices/ABBC0F72-8EA1-11D1-00A0-C90629100000 ]] || die 'ambient-light WMI device missing'
command -v dkms >/dev/null || die 'install dkms and matching linux-headers first'
kernel_release=$(uname -r)
[[ -f /lib/modules/$kernel_release/build/Makefile ]] || die 'matching kernel headers missing'
source_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../brightness/als" && pwd)
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$(dirname -- "${BASH_SOURCE[0]}")/../Cargo.toml" | head -1)
[[ -n $version ]] || die 'could not read AORUS Control version'
destination=/usr/src/aorus-als-$version
# Never overwrite the source of a registered DKMS version with different code.
for file in aorus-als.c Makefile; do
  if [[ -e $destination/$file ]]; then
    cmp -s "$source_dir/$file" "$destination/$file" || die "existing $destination/$file differs; a new DKMS version is required"
  fi
done
install -d -m 0755 "$destination"
for file in aorus-als.c Makefile dkms.conf; do
  if [[ $file == dkms.conf ]]; then
    sed "s/#MODULE_VERSION#/$version/g" "$source_dir/$file" > "$destination/$file"
  else
    install -m 0644 "$source_dir/$file" "$destination/$file"
  fi
done
if [[ -z $(dkms status -m aorus-als -v "$version") ]]; then
  dkms add -m aorus-als -v "$version"
fi
dkms install -m aorus-als -v "$version" -k "$kernel_release"
# modprobe leaves an already-loaded module in place; no unload or rebind.
modprobe aorus-als
found=false
for device in /sys/bus/iio/devices/iio:device*; do
  [[ -f $device/name ]] || continue
  [[ $(cat "$device/name") == aorus-ambient-light ]] || continue
  found=true
  printf 'Ambient-light sensor: %s\n' "$device"
  if ! cat "$device/in_illuminance_input"; then
    printf 'Sensor registered; waiting for the first firmware light sample.\n'
  fi
done
[[ $found == true ]] || die 'module loaded but sensor did not bind; inspect journalctl -k for aorus-als errors'
install -d -m 0755 /etc/modules-load.d
printf 'aorus-als\n' > /etc/modules-load.d/aorus-als.conf
printf 'Ambient-light driver installed for boot and DKMS kernel updates. Automatic brightness remains opt-in.\n'
