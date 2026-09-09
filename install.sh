#!/usr/bin/env bash
set -euo pipefail

die() { printf 'install: %s\n' "$*" >&2; exit 1; }
root_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
prefix=${PREFIX:-/usr/local}
destdir=${DESTDIR:-}
[[ $prefix == /usr/local ]] || die 'PREFIX must remain /usr/local because packaged units use /usr/local paths'
if [[ -z $destdir && ${EUID:-$(id -u)} -ne 0 ]]; then
  die 'run as root when installing; this script never invokes sudo'
fi

for binary in aorusctl aorusd aorus-control aorus-auto-brightness; do
  [[ -x $root_dir/target/release/$binary ]] \
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
  70-aorus-brightness-hid-bpf.rules; do
  [[ -f $root_dir/packaging/$file ]] || die "missing packaging/$file"
done
[[ -f $root_dir/tools/exclusive-hardware-test.sh ]] \
  || die 'missing tools/exclusive-hardware-test.sh'
[[ -f $root_dir/tools/fn-identity-hid-bpf-test.sh ]] \
  || die 'missing tools/fn-identity-hid-bpf-test.sh'
[[ -f $root_dir/tools/fn-buttons-capture.sh ]] \
  || die 'missing tools/fn-buttons-capture.sh'

config_dir=$destdir/etc/aorus-control
etc_file=$config_dir/config.toml
mode_dropin=$destdir/etc/systemd/system/aorusd.service.d/mode.conf
if [[ -e $mode_dropin ]] && ! grep -Fqx 'ExecStart=/usr/local/libexec/aorusd --write-enabled' "$mode_dropin"; then
  die "unexpected Rust daemon override at $mode_dropin; refusing to replace it"
fi
if [[ -z $destdir && -e /etc/aorus-control/brightness-hid-bpf.enabled ]]; then
  [[ -f $root_dir/target/aorus-brightness.bpf.o ||
     -f /usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o ]] ||
    die 'native Fn keys are enabled but target/aorus-brightness.bpf.o is missing'
  [[ -x $root_dir/target/udev-hid-bpf/target/release/udev-hid-bpf ||
     -x /usr/local/libexec/aorus-udev-hid-bpf ]] ||
    command -v udev-hid-bpf >/dev/null ||
    die 'native Fn keys are enabled but udev-hid-bpf is missing'
fi
install -d -m 0700 "$config_dir"
if [[ -e $etc_file ]]; then
  printf '%s\n' "Preserving existing $etc_file"
else
  install -D -m 0600 "$root_dir/packaging/aorus-control.toml" "$etc_file"
fi

install -D -m 0755 "$root_dir/target/release/aorusctl" "$destdir/usr/local/bin/aorusctl"
install -D -m 0755 "$root_dir/target/release/aorus-control" "$destdir/usr/local/bin/aorus-control"
install -D -m 0755 "$root_dir/target/release/aorusd" "$destdir/usr/local/libexec/aorusd"
install -D -m 0755 "$root_dir/target/release/aorus-auto-brightness" \
  "$destdir/usr/local/libexec/aorus-auto-brightness"
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
install -D -m 0755 "$root_dir/packaging/migrate-to-rust.sh" \
  "$destdir/usr/local/libexec/aorus-control-migrate-to-rust"
install -D -m 0755 "$root_dir/packaging/rollback-to-python.sh" \
  "$destdir/usr/local/libexec/aorus-control-rollback-to-python"
install -D -m 0755 "$root_dir/tools/exclusive-hardware-test.sh" \
  "$destdir/usr/local/libexec/aorus-control-exclusive-hardware-test"
install -D -m 0755 "$root_dir/tools/fn-identity-hid-bpf-test.sh" \
  "$destdir/usr/local/libexec/aorus-control-fn-identity-test"
install -D -m 0755 "$root_dir/tools/fn-buttons-capture.sh" \
  "$destdir/usr/local/libexec/aorus-control-fn-buttons-capture"
install -D -m 0755 "$root_dir/packaging/aorus-brightness-hid-bpf" \
  "$destdir/usr/local/libexec/aorus-brightness-hid-bpf"
install -D -m 0644 "$root_dir/packaging/70-aorus-brightness-hid-bpf.rules" \
  "$destdir/usr/lib/udev/rules.d/70-aorus-brightness-hid-bpf.rules"
if [[ -f $root_dir/target/aorus-brightness.bpf.o ]]; then
  install -D -m 0644 "$root_dir/target/aorus-brightness.bpf.o" \
    "$destdir/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o"
fi
# Remove the retired duplicate prototype; the production object now contains
# only the capture-proven Fn identities.
rm -f "$destdir/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5-fn-identity-prototype.bpf.o"
if [[ -x $root_dir/target/udev-hid-bpf/target/release/udev-hid-bpf ]]; then
  install -D -m 0755 "$root_dir/target/udev-hid-bpf/target/release/udev-hid-bpf" \
    "$destdir/usr/local/libexec/aorus-udev-hid-bpf"
fi

# Remove the retired userspace input bridge. It is never a fallback for the
# native kernel HID path.
rm -f \
  "$destdir/usr/local/libexec/aorus-hotkey-bridge" \
  "$destdir/usr/lib/systemd/user/aorus-hotkey-bridge.service" \
  "$destdir/usr/lib/udev/rules.d/70-aorus-hotkey-bridge.rules"

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
      /usr/local/libexec/aorus-brightness-hid-bpf add "$device"
    done
  fi
fi

if [[ -z $destdir && -e /etc/aorus-control/brightness-hid-bpf.enabled ]]; then
  brightness_status='Native HID Fn-key support is enabled.'
else
  brightness_status='Native HID Fn-key support remains disabled until enabled in the app.'
fi
if [[ -e $mode_dropin ]]; then
  daemon_status='Updated AORUS Control with existing Rust write ownership preserved.'
  ownership_status='Rust remains the sole persistent fan-control writer.'
else
  daemon_status='Installed AORUS Control in shadow/read-only mode.'
  ownership_status='Persistent Rust write ownership requires the explicit migrate-to-rust operation.'
fi
printf '%s\n' \
  "$daemon_status" \
  'Automatic brightness is installed but remains disabled until enabled in the app.' \
  "$brightness_status" \
  'The Python profile-sync service was not stopped, disabled, or modified.' \
  "$ownership_status"
