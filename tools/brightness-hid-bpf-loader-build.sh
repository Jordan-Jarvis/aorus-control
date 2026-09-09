#!/usr/bin/env bash
set -euo pipefail

die() {
  printf 'brightness-hid-bpf-loader-build: %s\n' "$*" >&2
  exit 2
}

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
source_dir="$root/target/udev-hid-bpf"

# The object build also fetches the pinned upstream loader source.
"$root/tools/brightness-hid-bpf-build.sh"
pkg-config --exists libudev 2>/dev/null ||
  die 'libudev development files are required (sudo apt install libudev-dev libelf-dev)'

MESON_BINDIR=/usr/local/libexec \
BPF_LOOKUP_DIRS=/usr/local/lib/aorus-control \
  cargo build --release --locked --manifest-path "$source_dir/Cargo.toml"
printf 'Built %s\n' "$source_dir/target/release/udev-hid-bpf"
