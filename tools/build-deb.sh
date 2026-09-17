#!/usr/bin/env bash
set -euo pipefail

die() { printf 'build-deb: %s\n' "$*" >&2; exit 2; }
root=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
out=${1:-$root/dist}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -1)
[[ -n $version ]] || die 'could not read version from Cargo.toml'
command -v dpkg-deb >/dev/null || die 'dpkg-deb is required'
mkdir -p "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
bpf_object="$tmp/aorus-brightness.bpf.o"
CARGO_TARGET_DIR="$root/target/deb" AORUS_INSTALL_PREFIX=/usr \
  cargo build --release --locked --manifest-path "$root/Cargo.toml"
"$root/tools/brightness-hid-bpf-build.sh" "$bpf_object"
source_dir="$root/target/udev-hid-bpf"
MESON_BINDIR=/usr/libexec BPF_LOOKUP_DIRS=/usr/lib/aorus-control \
  CARGO_TARGET_DIR="$source_dir/target/deb" cargo build --release --locked --manifest-path "$source_dir/Cargo.toml"
stage=$tmp/root
mkdir -p "$stage"
PREFIX=/usr BINDIR=/usr/bin LIBEXECDIR=/usr/libexec LIBDIR=/usr/lib/aorus-control \
  TARGET_DIR="$root/target/deb" LOADER_BINARY="$source_dir/target/deb/release/udev-hid-bpf" \
  BPF_OBJECT="$bpf_object" DESTDIR="$stage" "$root/install.sh" >/dev/null
# Runtime scripts and unit files are text templates retained by install.sh.
# Render their paths for the Debian filesystem layout.
find "$stage" -type f \
  \( -name '*.service' -o -name '*.desktop' -o -name '*.rules' -o -name 'aorus-brightness-hid-bpf' \) \
  -exec sed -i \
    -e 's#/usr/local/bin#/usr/bin#g' \
    -e 's#/usr/local/libexec#/usr/libexec#g' \
    -e 's#/usr/local/lib/aorus-control#/usr/lib/aorus-control#g' {} +
install -D -m 0644 "$root/LICENSE" "$stage/usr/share/doc/aorus-control/copyright"
install -D -m 0644 "$root/THIRD-PARTY-NOTICES.md" "$stage/usr/share/doc/aorus-control/THIRD-PARTY-NOTICES.md"
install -D -m 0644 "$root/COPYING.GPL-2.0" "$stage/usr/share/doc/aorus-control/COPYING.GPL-2.0"

make_pkg() {
  local name=$1 arch=$2 description=$3
  local pkg="$tmp/$name"
  mkdir -p "$pkg/DEBIAN"
  cp -a "$stage/." "$pkg/"
  cat >"$pkg/DEBIAN/control" <<CONTROL
Package: $name
Version: $version-1
Section: utils
Priority: optional
Architecture: $arch
Maintainer: AORUS Control contributors
Depends: libc6, libgcc-s1, libcap2, libelf1, libudev1, libzstd1, zlib1g, libgl1, libegl1, libx11-6, libxkbcommon0, libwayland-client0, systemd, udev, dbus, policykit-1, dkms, build-essential, linux-headers-generic
Description: Native Linux controls for supported GIGABYTE AERO/AORUS laptops
 $description
CONTROL
  printf '%s\n' '/etc/aorus-control/config.toml' >"$pkg/DEBIAN/conffiles"
  cat >"$pkg/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules >/dev/null 2>&1 || true
fi
if [ -x /usr/libexec/aorus-driver-install ]; then
    if ! /usr/libexec/aorus-driver-install --package-install; then
        echo 'Warning: the AORUS WMI driver could not be installed automatically; use Hardware / Diagnostics in AORUS Control.' >&2
    fi
else
    echo 'Warning: the bundled AORUS WMI driver installer is unavailable.' >&2
fi
if command -v busctl >/dev/null 2>&1 && [ -S /run/dbus/system_bus_socket ]; then
    busctl call --system org.freedesktop.DBus /org/freedesktop/DBus \
        org.freedesktop.DBus ReloadConfig >/dev/null 2>&1 || true
fi
if command -v deb-systemd-helper >/dev/null 2>&1; then
    deb-systemd-helper unmask aorusd.service >/dev/null 2>&1 || true
    deb-systemd-helper enable aorusd.service >/dev/null 2>&1 || true
fi
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload >/dev/null 2>&1 || true
    if [ -n "${2:-}" ]; then
        deb-systemd-invoke restart aorusd.service || true
    else
        deb-systemd-invoke start aorusd.service || true
    fi
fi
if [ "$(cat /sys/class/dmi/id/sys_vendor 2>/dev/null || true)" != GIGABYTE ] ||
   [ "$(cat /sys/class/dmi/id/product_name 2>/dev/null || true)" != 'AERO 16 YE5' ] ||
   [ "$(cat /sys/class/dmi/id/product_version 2>/dev/null || true)" != P86VE ]; then
    echo 'Warning: this laptop is not the verified AERO 16 YE5; hardware controls may be unavailable.' >&2
fi
exit 0
POSTINST
  chmod 0755 "$pkg/DEBIAN/postinst"
  cat >"$pkg/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
if [ "$1" = remove ] && command -v deb-systemd-invoke >/dev/null 2>&1; then
    deb-systemd-invoke stop aorusd.service >/dev/null 2>&1 || true
fi
if [ "$1" = remove ] && [ -x /usr/local/libexec/aorus-brightness-hid-bpf ]; then
    for device in /sys/bus/hid/devices/0003:1044:7A3A.*; do
        [ -e "$device" ] || continue
        [ "$(cat "$device/../bInterfaceNumber" 2>/dev/null || true)" = 02 ] || continue
        /usr/local/libexec/aorus-brightness-hid-bpf remove "$device" >/dev/null 2>&1 || true
    done
fi
if [ "$1" = remove ] && [ -e /var/lib/aorus-control/driver-managed ] && command -v dkms >/dev/null 2>&1; then
    version=$(sed -n 's/^PACKAGE_VERSION="\([^"]*\)"/\1/p' /usr/share/aorus-control/driver/aorus-laptop-dkms/dkms.conf)
    modprobe -r aorus_laptop >/dev/null 2>&1 || true
    if [ -n "$version" ]; then
        dkms remove -m aorus-laptop -v "$version" --all >/dev/null 2>&1 || true
    fi
    rm -f /etc/modules-load.d/aorus-laptop.conf /var/lib/aorus-control/driver-managed
    depmod -a >/dev/null 2>&1 || true
fi
exit 0
PRERM
  sed -i 's#/usr/local/libexec#/usr/libexec#g' "$pkg/DEBIAN/prerm"
  chmod 0755 "$pkg/DEBIAN/prerm"
  cat >"$pkg/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
if [ "$1" = purge ]; then
    deb-systemd-helper purge aorusd.service >/dev/null 2>&1 || true
    rm -f /var/lib/aorus-control/driver-managed
fi
if [ -d /run/systemd/system ]; then
    systemctl daemon-reload >/dev/null 2>&1 || true
fi
exit 0
POSTRM
  chmod 0755 "$pkg/DEBIAN/postrm"
  dpkg-deb --root-owner-group --build "$pkg" "$out/${name}_${version}-1_${arch}.deb" >/dev/null
}

make_pkg aorus-control amd64 'The daemon, native Rust UI, CLI, exact-model native HID-BPF Fn-key path, and system integration files.'

als="$tmp/aorus-control-als-dkms"
mkdir -p "$als/DEBIAN" "$als/usr/src/aorus-als-$version"
cp "$root/brightness/als/"{aorus-als.c,Makefile,dkms.conf} "$als/usr/src/aorus-als-$version/"
sed -i "s/#MODULE_VERSION#/$version/g" "$als/usr/src/aorus-als-$version/dkms.conf"
cat >"$als/DEBIAN/control" <<CONTROL
Package: aorus-control-als-dkms
Version: $version-1
Section: kernel
Priority: optional
Architecture: all
Maintainer: AORUS Control contributors
Depends: dkms
Description: Ambient-light DKMS module for AORUS Control
 Optional ambient-light sensor bridge for the verified GIGABYTE AERO 16 YE5.
CONTROL
install -D -m 0644 "$root/COPYING.GPL-2.0" "$als/usr/share/doc/aorus-control-als-dkms/copyright"
cat >"$als/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
version=__VERSION__
if command -v dkms >/dev/null 2>&1; then
    if [ -z "$(dkms status -m aorus-als -v "$version")" ]; then
        dkms add -m aorus-als -v "$version"
    fi
    if ! dkms install -m aorus-als -v "$version"; then
        echo 'Warning: ALS module was not built; install headers for the running kernel and run dkms autoinstall.' >&2
    elif [ -d /run/systemd/system ]; then
        modprobe aorus-als || echo 'Warning: ALS module could not load (unsupported hardware or Secure Boot policy).' >&2
    fi
fi
exit 0
POSTINST
# The module source is harmless when the optional kernel build prerequisites are absent.
sed -i "s/__VERSION__/$version/" "$als/DEBIAN/postinst"
chmod 0755 "$als/DEBIAN/postinst"
cat >"$als/DEBIAN/prerm" <<'PRERM'
#!/bin/sh
set -e
case "$1" in
    remove|upgrade|deconfigure)
        dkms remove -m aorus-als -v __VERSION__ --all || true
        ;;
esac
exit 0
PRERM
sed -i "s/__VERSION__/$version/" "$als/DEBIAN/prerm"
chmod 0755 "$als/DEBIAN/prerm"
dpkg-deb --root-owner-group --build "$als" "$out/aorus-control-als-dkms_${version}-1_all.deb" >/dev/null
printf 'Built packages in %s\n' "$out"
