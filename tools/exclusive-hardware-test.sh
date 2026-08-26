#!/usr/bin/env bash
set -Eeuo pipefail

readonly TRANSIENT_UNIT=aorusd-exclusive.service
readonly PYTHON_UNIT=aorus-power-profile-sync.service
readonly SHADOW_UNIT=aorusd.service
readonly PLATFORM=/sys/devices/platform/aorus_laptop

die() {
    printf 'exclusive-hardware-test: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
Usage: sudo ./tools/exclusive-hardware-test.sh --confirm-exclusive [-- COMMAND...]

Temporarily stop the Python writer and the installed Rust shadow daemon, run
aorusd as the sole write-enabled writer, then always restore Python. With no
COMMAND, the script waits for Enter so tests can be run from another terminal.

This is a Phase 1 test window, not the persistent service migration.
EOF
}

[[ ${1:-} == --confirm-exclusive ]] || {
    usage >&2
    exit 2
}
shift
if [[ ${1:-} == -- ]]; then
    shift
fi

[[ ${EUID:-$(id -u)} -eq 0 ]] || die 'run this procedure as root'
for command in systemctl systemd-run /usr/local/bin/aorusctl /usr/local/libexec/aorusd; do
    [[ -x $command ]] || command -v "$command" >/dev/null 2>&1 \
        || die "required command is missing: $command"
done
[[ -w $PLATFORM/fan_mode ]] || die "$PLATFORM/fan_mode is unavailable or not writable"
systemctl is-enabled --quiet "$PYTHON_UNIT" || die "$PYTHON_UNIT must be enabled before testing"
systemctl is-active --quiet "$PYTHON_UNIT" || die "$PYTHON_UNIT must be active before testing"
systemctl is-active --quiet "$SHADOW_UNIT" || die "$SHADOW_UNIT must be running in shadow mode"
/usr/local/bin/aorusctl status | grep -qx 'daemon_mode=shadow' \
    || die 'installed Rust daemon did not report shadow mode'
[[ ! -e /etc/systemd/system/aorusd.service.d/mode.conf ]] \
    || die 'persistent Rust write-mode drop-in exists; roll back before using this test window'

previous_mode=$(<"$PLATFORM/fan_mode")
case $previous_mode in
    0) previous_profile=normal ;;
    1) previous_profile=silent ;;
    2) previous_profile=gaming ;;
    3) previous_profile=custom ;;
    *) previous_profile=gaming ;;
esac

window_open=false
shadow_was_active=true

restore() {
    local result=$?
    trap - EXIT INT TERM
    set +e

    if [[ $window_open == true ]]; then
        if ! /usr/local/bin/aorusctl fan "$previous_profile" >/dev/null 2>&1; then
            # Gaming is a firmware profile, not a fixed fan speed. This direct
            # fallback is used only if the sole writer daemon cannot respond.
            printf '2' >"$PLATFORM/fan_mode" 2>/dev/null || true
        fi
    fi
    systemctl stop "$TRANSIENT_UNIT" >/dev/null 2>&1 || true
    systemctl reset-failed "$TRANSIENT_UNIT" >/dev/null 2>&1 || true
    systemctl enable --now "$PYTHON_UNIT" >/dev/null 2>&1 || true
    if [[ $shadow_was_active == true ]]; then
        systemctl start "$SHADOW_UNIT" >/dev/null 2>&1 || true
    fi
    /usr/local/libexec/aorus-power-profile-sync force >/dev/null 2>&1 || true

    if ! systemctl is-active --quiet "$PYTHON_UNIT"; then
        printf 'CRITICAL: Python fan-profile service was not restored; select Gaming and inspect systemctl/journalctl immediately.\n' >&2
        exit 1
    fi
    printf 'Exclusive window closed: Python is active again; persistent Rust ownership was not enabled.\n'
    exit "$result"
}
trap restore EXIT INT TERM

systemctl stop "$SHADOW_UNIT"
systemctl stop "$PYTHON_UNIT"
systemd-run --system --unit=aorusd-exclusive --collect --property=Type=simple \
    --property=Conflicts="$PYTHON_UNIT" \
    /usr/local/libexec/aorusd --write-enabled

for _ in {1..20}; do
    if /usr/local/bin/aorusctl status 2>/dev/null | grep -qx 'daemon_mode=write-enabled'; then
        window_open=true
        break
    fi
    sleep 0.5
done
[[ $window_open == true ]] || die 'transient Rust writer did not become ready'

printf 'Exclusive Rust write window is open. Python is stopped; no persistent cutover was made.\n'
if (($#)); then
    "$@"
else
    printf 'Run the planned aorusctl tests from another terminal, then press Enter here to restore Python.\n'
    read -r
fi
