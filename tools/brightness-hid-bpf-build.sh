#!/usr/bin/env bash
set -euo pipefail

die() {
  printf 'brightness-hid-bpf-build: %s\n' "$*" >&2
  exit 2
}

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
source_file="$root/brightness/hid-bpf/0010-Gigabyte__AERO-16-YE5.bpf.c"
source_commit=899cae5d423d1bde12f204edbf24a7bc7b22434d
udev_hid_bpf=${UDEV_HID_BPF_SOURCE:-"$root/target/udev-hid-bpf"}
output=${1:-"$root/target/aorus-brightness.bpf.o"}

command -v clang >/dev/null || die 'clang is required'
command -v git >/dev/null || die 'git is required'

if [[ ! -f $udev_hid_bpf/src/bpf/vmlinux.h ]]; then
  [[ -z ${UDEV_HID_BPF_SOURCE:-} ]] || die "invalid UDEV_HID_BPF_SOURCE: $udev_hid_bpf"
  rm -rf -- "$udev_hid_bpf"
  git clone -q https://gitlab.freedesktop.org/libevdev/udev-hid-bpf.git "$udev_hid_bpf"
  git -C "$udev_hid_bpf" checkout -q "$source_commit"
fi

libbpf_include=
if pkg-config --exists libbpf 2>/dev/null; then
  libbpf_include=$(pkg-config --variable=includedir libbpf)
else
  candidate="/lib/modules/$(uname -r)/build/tools/bpf/resolve_btfids/libbpf/include"
  [[ -f $candidate/bpf/bpf_helpers.h ]] ||
    die 'libbpf headers are required (install libbpf-dev)'
  libbpf_include=$candidate
fi

triplet=$(gcc -dumpmachine 2>/dev/null || true)
[[ -n $triplet && -d /usr/include/$triplet ]] || die 'a native GCC include directory is required'
mkdir -p -- "$(dirname -- "$output")"

clang -std=gnu11 -fno-stack-protector -O2 -target bpf -g -c \
  -fms-extensions -Wno-microsoft-anon-tag -D__x86_64__ \
  -I"$udev_hid_bpf/src/bpf" \
  -isystem "/usr/include/$triplet" \
  -idirafter "$libbpf_include" \
  "$source_file" -o "$output"

printf 'Built %s\n' "$output"
