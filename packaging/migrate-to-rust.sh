#!/usr/bin/env bash
set -euo pipefail

die() { printf 'migrate-to-rust: %s\n' "$*" >&2; exit 1; }

if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  die 'run as root for the explicit cutover (the normal installer does not need this script)'
fi
if [[ ${1:-} != --confirm-rust-write ]]; then
  printf '%s\n' \
    'This changes fan-control ownership from Python to aorusd.' \
    'It stops and disables aorus-power-profile-sync.service.' \
    'Run only after Phase 1 shadow and exclusive hardware tests pass:' \
    "  sudo $0 --confirm-rust-write" >&2
  exit 2
fi

command -v systemctl >/dev/null 2>&1 || die 'systemctl is required'
[[ -x /usr/local/bin/aorusctl ]] || die '/usr/local/bin/aorusctl is missing; install the complete application first'
systemctl cat aorusd.service >/dev/null 2>&1 || die 'aorusd.service is not installed'
systemctl cat aorus-power-profile-sync.service >/dev/null 2>&1 || die 'Python fallback service is not installed'
systemctl is-enabled --quiet aorus-power-profile-sync.service || die 'Python fallback service is not enabled'
systemctl is-active --quiet aorus-power-profile-sync.service || die 'Python fallback service is not active'

mode_dropin=/etc/systemd/system/aorusd.service.d/mode.conf
[[ ! -e $mode_dropin ]] || die "$mode_dropin already exists; resolve it manually before migration"

systemctl daemon-reload
if systemctl is-active --quiet aorusd.service; then
  :
else
  systemctl start aorusd.service
fi

shadow_status=''
for _ in {1..10}; do
  shadow_status=$(/usr/local/bin/aorusctl status 2>/dev/null || true)
  if printf '%s\n' "$shadow_status" | grep -qx 'daemon_mode=shadow'; then
    break
  fi
  sleep 1
done
printf '%s\n' "$shadow_status" | grep -qx 'daemon_mode=shadow' \
  || die 'aorusd did not report shadow mode; refusing concurrent-writer cutover'

backup_dir=/var/lib/aorus-control/migration-backups/$(date -u +%Y%m%dT%H%M%SZ)
install -d -m 0700 "$backup_dir"
if [[ -e /etc/aorus-control/config.toml ]]; then
  install -m 0600 /etc/aorus-control/config.toml "$backup_dir/config.toml"
fi
systemctl show aorusd.service -p FragmentPath -p DropInPaths -p Environment >"$backup_dir/aorusd-systemd-state"
printf '%s\n' "Migration backup: $backup_dir"

rollback() {
  set +e
  rm -f "$mode_dropin"
  systemctl daemon-reload
  systemctl disable --now aorusd.service >/dev/null 2>&1
  systemctl enable --now aorus-power-profile-sync.service >/dev/null 2>&1
  printf '%s\n' 'Migration failed; Python fallback was re-enabled. Check systemctl/journalctl.' >&2
}
trap rollback ERR

install -d -m 0755 "${mode_dropin%/*}"
printf '%s\n' \
  '[Unit]' \
  'Conflicts=aorus-power-profile-sync.service' \
  '' \
  '[Service]' \
  'ExecStart=' \
  'ExecStart=/usr/local/libexec/aorusd --write-enabled' \
  >"$mode_dropin"
systemctl daemon-reload
systemctl stop aorusd.service
systemctl stop aorus-power-profile-sync.service
systemctl disable aorus-power-profile-sync.service
systemctl enable --now aorusd.service

write_status=$(/usr/local/bin/aorusctl status 2>/dev/null || true)
if ! printf '%s\n' "$write_status" | grep -qx 'daemon_mode=write-enabled'; then
  rollback
  trap - ERR
  exit 1
fi

trap - ERR
printf '%s\n' 'Rust daemon is now the sole persistent fan-control writer.'
printf '%s\n' 'Rollback: sudo /usr/local/libexec/aorus-control-rollback-to-python --confirm-python'
