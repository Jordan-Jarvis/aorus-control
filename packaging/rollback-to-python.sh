#!/usr/bin/env bash
set -euo pipefail

die() { printf 'rollback-to-python: %s\n' "$*" >&2; exit 1; }

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  die 'run as root to restore the Python service'
fi
if [[ ${1:-} != --confirm-python ]]; then
  printf '%s\n' \
    'This disables persistent Rust fan-control ownership and restores Python.' \
    "Run explicitly: sudo $0 --confirm-python" >&2
  exit 2
fi

command -v systemctl >/dev/null 2>&1 || die 'systemctl is required'
systemctl cat aorus-power-profile-sync.service >/dev/null 2>&1 \
  || die 'Python fallback service is not installed'

mode_dropin=/etc/systemd/system/aorusd.service.d/mode.conf
if systemctl is-active --quiet aorusd.service; then
  systemctl disable --now aorusd.service
fi
rm -f "$mode_dropin"
if [[ -d ${mode_dropin%/*} ]]; then
  rmdir "${mode_dropin%/*}" 2>/dev/null || true
fi
systemctl daemon-reload
systemctl enable --now aorus-power-profile-sync.service
systemctl is-active --quiet aorus-power-profile-sync.service \
  || die 'Python service did not become active; inspect journalctl before using fan controls'

printf '%s\n' 'Python profile-sync service is active again; Rust daemon write ownership is disabled.'
