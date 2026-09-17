#!/usr/bin/env bash
set -euo pipefail

die() { printf 'uninstall: %s\n' "$*" >&2; exit 1; }
prefix=${PREFIX:-/usr/local}
destdir=${DESTDIR:-}

if [[ -z $destdir && ${EUID:-$(id -u)} -ne 0 ]]; then
  die 'run as root when uninstalling; this script never invokes sudo'
fi
if [[ -z $destdir ]]; then
  command -v systemctl >/dev/null 2>&1 || die 'systemctl is required'
fi

# Support both source installs (/usr/local) and Debian installs (/usr).
prefixes=("$prefix" /usr)

if [[ -z $destdir ]]; then
  # These are project-owned services only. No unrelated processes are touched.
  for package in aorus-control aorus-control-als-dkms; do
    if dpkg-query -W -f='${db:Status-Status}' "$package" 2>/dev/null | grep -qx installed; then
      dpkg --purge "$package" >/dev/null 2>&1 || true
    fi
  done
  systemctl disable --now aorusd.service aorus-power-profile-sync.service \
    >/dev/null 2>&1 || true

  # Detach the exact-model HID-BPF link before removing its loader and object.
  for helper in \
    "$prefix/libexec/aorus-brightness-hid-bpf" \
    "/usr/local/libexec/aorus-brightness-hid-bpf" \
    "/usr/libexec/aorus-brightness-hid-bpf"; do
    [[ -x $helper ]] || continue
    for device in /sys/bus/hid/devices/0003:1044:7A3A.*; do
      [[ -e $device ]] || continue
      [[ $(cat "$device/../bInterfaceNumber" 2>/dev/null || true) == 02 ]] || continue
      "$helper" remove "$device" >/dev/null 2>&1 || true
    done
    break
  done

  # Remove only the two DKMS modules owned by AORUS Control. Generic Gigabyte
  # hardware modules, audio profiles, and USB support are intentionally left.
  for module in aorus_laptop aorus_als; do
    modprobe -r "$module" >/dev/null 2>&1 || true
  done
  for module in aorus-laptop aorus-als; do
    mapfile -t versions < <(
      dkms status -m "$module" 2>/dev/null |
        awk -F'[/,]' -v module="$module" \
          '$1 == module { gsub(/^ +| +$/, "", $2); print $2 }' |
        sort -u
    )
    for version in "${versions[@]}"; do
      [[ -n $version ]] || continue
      dkms remove -m "$module" -v "$version" --all >/dev/null 2>&1 || true
    done
  done
  rm -rf /var/lib/dkms/aorus-laptop /var/lib/dkms/aorus-als
  find /usr/src -maxdepth 1 -type d \
    \( -name 'aorus-laptop-*' -o -name 'aorus-als-*' \) \
    -exec rm -rf -- {} +
  find /lib/modules -type f \
    \( -name 'aorus-laptop.ko' -o -name 'aorus-laptop.ko.*' \
    -o -name 'aorus-als.ko' -o -name 'aorus-als.ko.*' \) \
    -delete 2>/dev/null || true
  depmod -a
fi

for tree in "${prefixes[@]}"; do
  rm -f \
    "$destdir$tree/bin/aorusctl" \
    "$destdir$tree/bin/aorus-control" \
    "$destdir$tree/libexec/aorusd" \
    "$destdir$tree/libexec/aorus-auto-brightness" \
    "$destdir$tree/libexec/aorus-driver-install" \
    "$destdir$tree/libexec/aorus-als-install" \
    "$destdir$tree/libexec/aorus-control-fn-buttons-capture" \
    "$destdir$tree/libexec/aorus-brightness-hid-bpf" \
    "$destdir$tree/libexec/aorus-udev-hid-bpf" \
    "$destdir$tree/libexec/aorus-hotkey-bridge" \
    "$destdir$tree/libexec/aorus-power-profile-sync" \
    "$destdir$tree/libexec/aorus-control-exclusive-hardware-test" \
    "$destdir$tree/libexec/aorus-control-fn-identity-test" \
    "$destdir$tree/libexec/aorus-control-migrate-to-rust" \
    "$destdir$tree/libexec/aorus-control-rollback-to-python"
  rm -rf \
    "$destdir$tree/lib/aorus-control" \
    "$destdir$tree/share/aorus-control" \
    "$destdir$tree/share/doc/aorus-control"
done

rm -f \
  "$destdir/etc/aorus-control/brightness-hid-bpf.enabled" \
  "$destdir/etc/aorus-control/config.toml" \
  "$destdir/etc/aorus-power-profile-sync.conf" \
  "$destdir/etc/modules-load.d/aorus-laptop.conf" \
  "$destdir/etc/modules-load.d/aorus-als.conf" \
  "$destdir/etc/systemd/system/aorusd.service" \
  "$destdir/etc/systemd/system/aorus-power-profile-sync.service" \
  "$destdir/etc/systemd/system/aorusd.service.d/mode.conf" \
  "$destdir/etc/udev/hwdb.d/91-aorus-control.hwdb" \
  "$destdir/usr/lib/systemd/system/aorusd.service" \
  "$destdir/usr/lib/systemd/user/aorus-auto-brightness.service" \
  "$destdir/usr/lib/udev/rules.d/70-aorus-brightness-hid-bpf.rules" \
  "$destdir/usr/share/dbus-1/system.d/io.github.aoruslinux.Control1.conf" \
  "$destdir/usr/share/polkit-1/actions/io.github.aoruslinux.control.policy" \
  "$destdir/usr/share/applications/io.github.aoruslinux.Control.desktop" \
  "$destdir/etc/xdg/autostart/io.github.aoruslinux.Control.desktop" \
  "$destdir/usr/share/icons/hicolor/scalable/apps/io.github.aoruslinux.Control.svg"
rm -rf "$destdir/etc/aorus-control" "$destdir/var/lib/aorus-control"

if [[ -z $destdir ]]; then
  systemctl daemon-reload
  udevadm control --reload-rules
  systemd-hwdb update >/dev/null 2>&1 || true
  if command -v busctl >/dev/null 2>&1; then
    busctl call --system org.freedesktop.DBus /org/freedesktop/DBus \
      org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
  fi

  # Remove per-user state for the user who invoked sudo, not root's unrelated
  # configuration. Direct root invocation has no user state to clean.
  if [[ -n ${SUDO_USER:-} && ${SUDO_USER} != root ]]; then
    user_home=$(getent passwd "$SUDO_USER" | cut -d: -f6)
    if [[ -n $user_home && -d $user_home ]]; then
      rm -rf "$user_home/.config/aorus-control" \
        "$user_home/.cache/aorus-control" \
        "$user_home/.local/share/aorus-control"
    fi
  fi
fi

rmdir \
  "$destdir/etc/systemd/system/aorusd.service.d" \
  "$destdir/etc/aorus-control" \
  "$destdir/var/lib/aorus-control" \
  2>/dev/null || true

printf '%s\n' \
  'AORUS Control, its project services, custom drivers, and configuration were removed.' \
  'Unrelated audio, USB, and generic Gigabyte hardware support was left untouched.'
