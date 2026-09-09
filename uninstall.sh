#!/usr/bin/env bash
set -euo pipefail

die() { printf 'uninstall: %s\n' "$*" >&2; exit 1; }
destdir=${DESTDIR:-}
if [[ -z $destdir && ${EUID:-$(id -u)} -ne 0 ]]; then
  die 'run as root when uninstalling; this script never invokes sudo'
fi
if [[ -z $destdir ]]; then
  command -v systemctl >/dev/null 2>&1 || die 'systemctl is required'
fi

mode_dropin=$destdir/etc/systemd/system/aorusd.service.d/mode.conf
if [[ -e $mode_dropin ]]; then
  die 'Rust write mode is enabled; run /usr/local/libexec/aorus-control-rollback-to-python --confirm-python first'
fi

if [[ -z $destdir ]] && systemctl cat aorusd.service >/dev/null 2>&1; then
  if systemctl is-active --quiet aorusd.service; then
    systemctl disable --now aorusd.service
  else
    systemctl disable aorusd.service >/dev/null 2>&1 || true
  fi
  systemctl daemon-reload
fi

if [[ -z $destdir && -x /usr/local/libexec/aorus-brightness-hid-bpf ]]; then
  for device in /sys/bus/hid/devices/0003:1044:7A3A.*; do
    [[ -e $device ]] || continue
    [[ $(cat "$device/../bInterfaceNumber" 2>/dev/null || true) == 02 ]] || continue
    /usr/local/libexec/aorus-brightness-hid-bpf remove "$device" || true
  done
fi

rm -f \
  "$destdir/usr/local/bin/aorusctl" \
  "$destdir/usr/local/bin/aorus-control" \
  "$destdir/usr/local/libexec/aorusd" \
  "$destdir/usr/local/libexec/aorus-auto-brightness" \
  "$destdir/usr/local/libexec/aorus-control-migrate-to-rust" \
  "$destdir/usr/local/libexec/aorus-control-rollback-to-python" \
  "$destdir/usr/local/libexec/aorus-control-exclusive-hardware-test" \
  "$destdir/usr/local/libexec/aorus-control-fn-identity-test" \
  "$destdir/usr/local/libexec/aorus-control-fn-buttons-capture" \
  "$destdir/usr/local/libexec/aorus-brightness-hid-bpf" \
  "$destdir/usr/local/libexec/aorus-udev-hid-bpf" \
  "$destdir/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o" \
  "$destdir/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5-fn-identity-prototype.bpf.o" \
  "$destdir/etc/aorus-control/brightness-hid-bpf.enabled" \
  "$destdir/usr/lib/systemd/system/aorusd.service" \
  "$destdir/usr/lib/systemd/user/aorus-auto-brightness.service" \
  "$destdir/usr/lib/udev/rules.d/70-aorus-brightness-hid-bpf.rules" \
  "$destdir/usr/share/doc/aorus-control/auto-brightness.toml" \
  "$destdir/usr/share/dbus-1/system.d/io.github.aoruslinux.Control1.conf" \
  "$destdir/usr/share/polkit-1/actions/io.github.aoruslinux.control.policy" \
  "$destdir/usr/share/applications/io.github.aoruslinux.Control.desktop" \
  "$destdir/etc/xdg/autostart/io.github.aoruslinux.Control.desktop" \
  "$destdir/usr/share/icons/hicolor/scalable/apps/io.github.aoruslinux.Control.svg"

if [[ -z $destdir ]]; then
  systemctl daemon-reload
  udevadm control --reload-rules
fi
if [[ -z $destdir ]] && command -v busctl >/dev/null 2>&1; then
  busctl call --system org.freedesktop.DBus /org/freedesktop/DBus \
    org.freedesktop.DBus ReloadConfig
fi

# Deliberately preserve the configuration and state directories.
printf '%s\n' \
  'AORUS Control binaries and integration files removed.' \
  "Preserved $destdir/etc/aorus-control/config.toml and $destdir/var/lib/aorus-control." \
  'The Python profile-sync service was not stopped, disabled, or modified.'
