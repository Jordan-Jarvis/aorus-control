#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo fmt --all --check
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --all-features

while IFS= read -r script; do
  bash -n "$script"
done < <(find . -path ./target -prune -o -type f -name '*.sh' -print)

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
fi
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
DESTDIR=$stage ./install.sh
test -x "$stage/usr/local/libexec/aorusd"
test -f "$stage/etc/aorus-control/config.toml"
sed 's|^ExecStart=.*|ExecStart=/bin/true|' packaging/aorusd.service >"$stage/aorusd.service"
systemd-analyze verify "$stage/aorusd.service"
DESTDIR=$stage ./uninstall.sh
test ! -e "$stage/usr/local/libexec/aorusd"
test -f "$stage/etc/aorus-control/config.toml"
