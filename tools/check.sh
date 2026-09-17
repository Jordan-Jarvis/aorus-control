#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo fmt --all --check
if cargo tree --all-features -e features -i zbus | grep -F 'zbus feature "tokio"' >/dev/null; then
  printf '%s\n' 'zbus/tokio must stay disabled; it makes blocking daemon calls panic inside the D-Bus runtime' >&2
  exit 1
fi
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --all-features
target/release/aorusd --help | grep -Fqx \
  'Default mode is write-enabled; --shadow disables hardware mutations.'

while IFS= read -r script; do
  bash -n "$script"
done < <(find . -path ./target -prune -o -type f -name '*.sh' -print)
bash -n packaging/aorus-driver-install
grep -Fqx 'if [ -x /usr/libexec/aorus-driver-install ]; then' <(sed -n '/cat >"$pkg\/DEBIAN\/postinst"/,/^POSTINST/p' tools/build-deb.sh)

python3 - <<'PY'
import xml.etree.ElementTree as ET
for path in (
    "packaging/io.github.aoruslinux.Control1.conf",
    "packaging/io.github.aoruslinux.control.policy",
    "packaging/io.github.aoruslinux.Control.svg",
):
    ET.parse(path)
PY

if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate packaging/io.github.aoruslinux.Control.desktop
  desktop-file-validate packaging/io.github.aoruslinux.Control.Autostart.desktop
fi
if udevadm verify --help >/dev/null 2>&1; then
  udevadm verify packaging/70-aorus-brightness-hid-bpf.rules
fi
kernel_build=/lib/modules/$(uname -r)/build
if [[ -f $kernel_build/Makefile && -f $kernel_build/Module.symvers ]] &&
  grep -qw iio_device_alloc "$kernel_build/Module.symvers"; then
  make -C brightness/als W=1 check
  make -C brightness/als clean
else
  printf '%s\n' 'Skipping ambient-light module build: usable IIO kernel build metadata is unavailable.'
fi
if [[ -n ${UDEV_HID_BPF_SOURCE:-} ]]; then
  tools/brightness-hid-bpf-build.sh target/aorus-brightness.bpf.o
fi
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
DESTDIR=$stage ./install.sh
test -x "$stage/usr/local/libexec/aorusd"
test -x "$stage/usr/local/libexec/aorus-auto-brightness"
test -f "$stage/usr/lib/systemd/user/aorus-auto-brightness.service"
test -x "$stage/usr/local/libexec/aorus-brightness-hid-bpf"
test -x "$stage/usr/local/libexec/aorus-control-fn-buttons-capture"
test -f "$stage/usr/lib/udev/rules.d/70-aorus-brightness-hid-bpf.rules"
test -f "$stage/etc/xdg/autostart/io.github.aoruslinux.Control.desktop"
if [[ -f target/aorus-brightness.bpf.o ]]; then
  test -f "$stage/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o"
fi
test -f "$stage/etc/aorus-control/config.toml"
sed 's|^ExecStart=.*|ExecStart=/bin/true|' packaging/aorusd.service >"$stage/aorusd.service"
systemd-analyze verify "$stage/aorusd.service"
sed 's|^ExecStart=.*|ExecStart=/bin/true|' packaging/aorus-auto-brightness.service \
  >"$stage/aorus-auto-brightness.service"
systemd-analyze --user verify "$stage/aorus-auto-brightness.service"
DESTDIR=$stage ./uninstall.sh
test ! -e "$stage/usr/local/libexec/aorusd"
test ! -e "$stage/usr/local/libexec/aorus-auto-brightness"
test ! -e "$stage/usr/local/libexec/aorus-brightness-hid-bpf"
test ! -e "$stage/usr/local/libexec/aorus-control-fn-buttons-capture"
test ! -e "$stage/etc/xdg/autostart/io.github.aoruslinux.Control.desktop"
test -f "$stage/etc/aorus-control/config.toml"
