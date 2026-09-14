#!/usr/bin/env bash
set -euo pipefail

die() { printf 'install: %s\n' "$*" >&2; exit 1; }
root_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
prefix=${PREFIX:-/usr/local}
bindir=${BINDIR:-$prefix/bin}
libexecdir=${LIBEXECDIR:-$prefix/libexec}
libdir=${LIBDIR:-$prefix/lib/aorus-control}
target_dir=${TARGET_DIR:-$root_dir/target}
loader_binary=${LOADER_BINARY:-$target_dir/udev-hid-bpf/target/release/udev-hid-bpf}
bpf_object=${BPF_OBJECT:-$target_dir/aorus-brightness.bpf.o}
destdir=${DESTDIR:-}
if [[ -z $destdir && ${EUID:-$(id -u)} -ne 0 ]]; then
  die 'run as root when installing; this script never invokes sudo'
fi

for binary in aorusctl aorusd aorus-control aorus-auto-brightness; do
  [[ -x $target_dir/release/$binary ]] \
    || die "missing target/release/$binary; build with: cargo build --release"
done
for file in \
  aorusd.service \
  io.github.aoruslinux.Control1.conf \
  io.github.aoruslinux.control.policy \
  io.github.aoruslinux.Control.desktop \
  io.github.aoruslinux.Control.Autostart.desktop \
  io.github.aoruslinux.Control.svg \
  aorus-control.toml \
  aorus-auto-brightness.service \
  auto-brightness.toml \
  aorus-brightness-hid-bpf \
  aorus-driver-install \
  aorus-als-install \
  70-aorus-brightness-hid-bpf.rules; do
  [[ -f $root_dir/packaging/$file ]] || die "missing packaging/$file"
done
[[ -f $root_dir/tools/fn-buttons-capture.sh ]] \
  || [[ -n $destdir ]] \
  || die 'missing tools/fn-buttons-capture.sh'

config_dir=$destdir/etc/aorus-control
etc_file=$config_dir/config.toml
mode_dropin=$destdir/etc/systemd/system/aorusd.service.d/mode.conf
if [[ -z $destdir && -e /etc/aorus-control/brightness-hid-bpf.enabled ]]; then
  [[ -f $bpf_object ||
     -f "$libdir/0010-Gigabyte__AERO-16-YE5.bpf.o" ]] ||
    die 'native Fn keys are enabled but target/aorus-brightness.bpf.o is missing'
  [[ -x $loader_binary ||
     -x "$libexecdir/aorus-udev-hid-bpf" ]] ||
    command -v udev-hid-bpf >/dev/null ||
    die 'native Fn keys are enabled but udev-hid-bpf is missing'
fi
install -d -m 0700 "$config_dir"
install -D -m 0644 /dev/null "$config_dir/brightness-hid-bpf.enabled"
if [[ -e $etc_file ]]; then
  printf '%s\n' "Preserving existing $etc_file"
else
  install -D -m 0600 "$root_dir/packaging/aorus-control.toml" "$etc_file"
fi

install -D -m 0755 "$target_dir/release/aorusctl" "$destdir$bindir/aorusctl"
install -D -m 0755 "$target_dir/release/aorus-control" "$destdir$bindir/aorus-control"
install -D -m 0755 "$target_dir/release/aorusd" "$destdir$libexecdir/aorusd"
install -D -m 0755 "$target_dir/release/aorus-auto-brightness" \
  "$destdir$libexecdir/aorus-auto-brightness"
install -D -m 0644 "$root_dir/packaging/aorusd.service" "$destdir/usr/lib/systemd/system/aorusd.service"
install -D -m 0644 "$root_dir/packaging/aorus-auto-brightness.service" \
  "$destdir/usr/lib/systemd/user/aorus-auto-brightness.service"
install -D -m 0644 "$root_dir/packaging/auto-brightness.toml" \
  "$destdir/usr/share/doc/aorus-control/auto-brightness.toml"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.Control1.conf" \
  "$destdir/usr/share/dbus-1/system.d/io.github.aoruslinux.Control1.conf"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.control.policy" \
  "$destdir/usr/share/polkit-1/actions/io.github.aoruslinux.control.policy"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.Control.desktop" \
  "$destdir/usr/share/applications/io.github.aoruslinux.Control.desktop"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.Control.Autostart.desktop" \
  "$destdir/etc/xdg/autostart/io.github.aoruslinux.Control.desktop"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.Control.svg" \
  "$destdir/usr/share/icons/hicolor/scalable/apps/io.github.aoruslinux.Control.svg"
if [[ -f $root_dir/tools/fn-buttons-capture.sh ]]; then
  install -D -m 0755 "$root_dir/tools/fn-buttons-capture.sh" \
    "$destdir$libexecdir/aorus-control-fn-buttons-capture"
fi
install -D -m 0755 "$root_dir/packaging/aorus-brightness-hid-bpf" \
  "$destdir$libexecdir/aorus-brightness-hid-bpf"
install -D -m 0755 "$root_dir/packaging/aorus-driver-install" \
  "$destdir$libexecdir/aorus-driver-install"
install -D -m 0755 "$root_dir/packaging/aorus-als-install" \
  "$destdir$libexecdir/aorus-als-install"
for driver_file in Makefile aorus-laptop.c aorus-laptop.conf dkms.conf LICENSE README.md; do
  install -D -m 0644 "$root_dir/drivers/aorus-laptop-dkms/$driver_file" \
    "$destdir$prefix/share/aorus-control/driver/aorus-laptop-dkms/$driver_file"
done
for als_file in aorus-als.c Makefile dkms.conf; do
  install -D -m 0644 "$root_dir/brightness/als/$als_file" \
    "$destdir$prefix/share/aorus-control/driver/als/$als_file"
done
install -D -m 0644 "$root_dir/packaging/70-aorus-brightness-hid-bpf.rules" \
  "$destdir/usr/lib/udev/rules.d/70-aorus-brightness-hid-bpf.rules"
if [[ -f $bpf_object ]]; then
  install -D -m 0644 "$bpf_object" \
    "$destdir$libdir/0010-Gigabyte__AERO-16-YE5.bpf.o"
fi
# Remove the retired duplicate prototype; the production object now contains
# only the capture-proven Fn identities.
rm -f "$destdir$libdir/0010-Gigabyte__AERO-16-YE5-fn-identity-prototype.bpf.o"
if [[ -x $loader_binary ]]; then
  install -D -m 0755 "$loader_binary" \
    "$destdir$libexecdir/aorus-udev-hid-bpf"
fi

# Remove the retired userspace input bridge. It is never a fallback for the
# native kernel HID path.
rm -f \
  "$destdir$libexecdir/aorus-hotkey-bridge" \
  "$destdir/usr/lib/systemd/user/aorus-hotkey-bridge.service" \
  "$destdir/usr/lib/udev/rules.d/70-aorus-hotkey-bridge.rules"

# The packaged service now uses the daemon's write-enabled default directly.
rm -f "$mode_dropin"
rmdir "$destdir/etc/systemd/system/aorusd.service.d" 2>/dev/null || true

if [[ -z $destdir ]]; then
  systemctl daemon-reload
  udevadm control --reload-rules
  command -v busctl >/dev/null 2>&1 || die 'busctl is required to reload the system D-Bus policy'
  busctl call --system org.freedesktop.DBus /org/freedesktop/DBus \
    org.freedesktop.DBus ReloadConfig
  systemctl enable aorusd.service
  systemctl restart aorusd.service
  if [[ -e /etc/aorus-control/brightness-hid-bpf.enabled ]]; then
    for device in /sys/bus/hid/devices/0003:1044:7A3A.*; do
      [[ -e $device ]] || continue
      [[ $(cat "$device/../bInterfaceNumber" 2>/dev/null || true) == 02 ]] || continue
      "$libexecdir/aorus-brightness-hid-bpf" add "$device"
    done
  fi
fi

brightness_status='Native HID Fn-key support is enabled by default.'
printf '%s\n' \
  'Installed AORUS Control with write-enabled hardware control.' \
  'Automatic brightness is installed but remains disabled until enabled in the app.' \
  "$brightness_status"
