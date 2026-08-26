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

rm -f \
  "$destdir/usr/local/bin/aorusctl" \
  "$destdir/usr/local/bin/aorus-control" \
  "$destdir/usr/local/libexec/aorusd" \
  "$destdir/usr/local/libexec/aorus-control-migrate-to-rust" \
  "$destdir/usr/local/libexec/aorus-control-rollback-to-python" \
  "$destdir/usr/local/libexec/aorus-control-exclusive-hardware-test" \
  "$destdir/usr/lib/systemd/system/aorusd.service" \
  "$destdir/usr/share/dbus-1/system.d/io.github.aoruslinux.Control1.conf" \
  "$destdir/usr/share/polkit-1/actions/io.github.aoruslinux.control.policy" \
  "$destdir/usr/share/applications/io.github.aoruslinux.Control.desktop" \
  "$destdir/usr/share/icons/hicolor/scalable/apps/io.github.aoruslinux.Control.svg"

if [[ -z $destdir ]]; then
  systemctl daemon-reload
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
