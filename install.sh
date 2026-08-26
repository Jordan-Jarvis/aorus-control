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

for binary in aorusctl aorusd aorus-control; do
  [[ -x $root_dir/target/release/$binary ]] \
    || die "missing target/release/$binary; build with: cargo build --release"
done
for file in \
  aorusd.service \
  io.github.aoruslinux.Control1.conf \
  io.github.aoruslinux.control.policy \
  io.github.aoruslinux.Control.desktop \
  io.github.aoruslinux.Control.svg \
  aorus-control.toml; do
  [[ -f $root_dir/packaging/$file ]] || die "missing packaging/$file"
done
[[ -f $root_dir/tools/exclusive-hardware-test.sh ]] \
  || die 'missing tools/exclusive-hardware-test.sh'

config_dir=$destdir/etc/aorus-control
etc_file=$config_dir/config.toml
if [[ -e $destdir/etc/systemd/system/aorusd.service.d/mode.conf ]]; then
  die 'Rust write-mode override exists; run /usr/local/libexec/aorus-control-rollback-to-python --confirm-python before reinstalling'
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
install -D -m 0644 "$root_dir/packaging/aorusd.service" "$destdir/usr/lib/systemd/system/aorusd.service"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.Control1.conf" \
  "$destdir/usr/share/dbus-1/system.d/io.github.aoruslinux.Control1.conf"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.control.policy" \
  "$destdir/usr/share/polkit-1/actions/io.github.aoruslinux.control.policy"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.Control.desktop" \
  "$destdir/usr/share/applications/io.github.aoruslinux.Control.desktop"
install -D -m 0644 "$root_dir/packaging/io.github.aoruslinux.Control.svg" \
  "$destdir/usr/share/icons/hicolor/scalable/apps/io.github.aoruslinux.Control.svg"
install -D -m 0755 "$root_dir/packaging/migrate-to-rust.sh" \
  "$destdir/usr/local/libexec/aorus-control-migrate-to-rust"
install -D -m 0755 "$root_dir/packaging/rollback-to-python.sh" \
  "$destdir/usr/local/libexec/aorus-control-rollback-to-python"
install -D -m 0755 "$root_dir/tools/exclusive-hardware-test.sh" \
  "$destdir/usr/local/libexec/aorus-control-exclusive-hardware-test"

if [[ -z $destdir ]]; then
  systemctl daemon-reload
  command -v busctl >/dev/null 2>&1 || die 'busctl is required to reload the system D-Bus policy'
  busctl call --system org.freedesktop.DBus /org/freedesktop/DBus \
    org.freedesktop.DBus ReloadConfig
  systemctl enable aorusd.service
  systemctl restart aorusd.service
fi

printf '%s\n' \
  'Installed AORUS Control in shadow/read-only mode.' \
  'The Python profile-sync service was not stopped, disabled, or modified.' \
  'Persistent Rust write ownership requires the explicit migrate-to-rust operation.'
