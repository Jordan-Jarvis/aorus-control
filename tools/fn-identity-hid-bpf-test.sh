#!/usr/bin/env bash
set -Eeuo pipefail

die() {
    printf 'fn-identity-hid-bpf-test: %s\n' "$*" >&2
    exit 2
}

[[ ${EUID:-$(id -u)} -eq 0 ]] || die 'run this guarded test with sudo'
[[ $# -eq 1 && $1 == --confirm-external-keyboard ]] ||
    die 'connect an external keyboard, then pass --confirm-external-keyboard'

for command in evtest python3 sha256sum stat systemd-run systemctl timeout; do
    command -v "$command" >/dev/null || die "$command is required"
done

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

# Refuse before touching HID state unless the desktop user's COSMIC file
# already contains one AORUS-owned, modifierless identity mapping for every
# identity this production object can emit. The action is intentionally not checked:
# users may customize it in the application.
desktop_uid=${SUDO_UID:-}
desktop_user=${SUDO_USER:-}
[[ $desktop_uid =~ ^[1-9][0-9]*$ && -n $desktop_user ]] ||
    die 'the invoking desktop user could not be determined; run through sudo'
[[ $(id -u "$desktop_user" 2>/dev/null || true) == "$desktop_uid" ]] ||
    die 'SUDO_USER and SUDO_UID do not identify the same desktop user'
desktop_home=$(getent passwd "$desktop_uid" | awk -F: 'NR == 1 { print $6 }')
desktop_gid=$(id -g "$desktop_user")
[[ -n $desktop_home && -n $desktop_gid ]] || die "no passwd entry for UID $desktop_uid"
config_home=${SUDO_XDG_CONFIG_HOME:-${XDG_CONFIG_HOME:-}}
[[ -n $config_home && $config_home != /root* ]] || config_home="$desktop_home/.config"
cosmic_custom="$config_home/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom"
[[ -f $cosmic_custom ]] ||
    die "COSMIC AORUS Fn identity mappings are missing for $desktop_user: $cosmic_custom"
[[ $(stat -c '%u' "$cosmic_custom") == "$desktop_uid" ]] ||
    die "COSMIC shortcut file is not owned by $desktop_user: $cosmic_custom"
python3 - "$cosmic_custom" <<'PY'
import re
import sys

text = open(sys.argv[1], encoding="utf-8").read()
expected = {
    "XF86Tools": "brightness-down",  # KEY_F13 through XKB inet(evdev)
    "XF86Launch5": "brightness-up",  # KEY_F14
    "XF86Launch6": "fan",            # KEY_F15
    "XF86Launch7": "sleep",          # KEY_F16
    "XF86Launch8": "wifi",           # KEY_F17
    "F19": "square-x",
    "F22": "ai",
}
for key, button in expected.items():
    pattern = (
        rf'''\(\s*modifiers\s*:\s*\[\s*\]\s*,\s*key\s*:\s*'''
        rf'''(?:Some\()?"{key}"\)?\s*,\s*'''
        rf'''description\s*:\s*(?:Some\()?"AORUS Control: fn-button:{re.escape(button)}"\)?'''
    )
    if len(re.findall(pattern, text, flags=re.DOTALL)) != 1:
        raise SystemExit(
            f"COSMIC AORUS-owned identity mapping for {key} is absent or ambiguous"
        )
PY

object=/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o
prototype="$root/target/aorus-brightness.bpf.o"
installed_prototype=/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o
if [[ ! -f $prototype && -f $installed_prototype ]]; then
    prototype=$installed_prototype
fi
if [[ -f $root/packaging/aorus-brightness-hid-bpf ]]; then
    helper=$root/packaging/aorus-brightness-hid-bpf
elif [[ -f /usr/local/libexec/aorus-brightness-hid-bpf ]]; then
    helper=/usr/local/libexec/aorus-brightness-hid-bpf
else
    die 'the existing brightness HID-BPF helper is not installed or in the source tree'
fi
[[ -f $object && ! -L $object ]] ||
    die "the existing brightness-only object is missing or is not a regular file: $object"
[[ -f $prototype && ! -L $prototype ]] ||
    die "missing production object: $prototype (build with tools/brightness-hid-bpf-build.sh)"

# SHA-256 values are for the exact HID report descriptors, not BPF ELF files.
# The production identity object appends 53 bytes to the 253-byte source descriptor.
source_descriptor_sha256=8c466c33cedbb3be04738089da3c09d4319443a793b462319708a8f8364be17a
brightness_descriptor_sha256=637f4bd5f31d413593ab1095e97f0567b4456c04e78bdb1268fc641e3e9e1a48
prototype_descriptor_sha256=7c69146eea1d52d72015cdcc225e462e7110d4e5f8b26142986231c24a4f8271

[[ $(cat /sys/class/dmi/id/sys_vendor 2>/dev/null || true) == GIGABYTE &&
   $(cat /sys/class/dmi/id/product_name 2>/dev/null || true) == 'AERO 16 YE5' &&
   $(cat /sys/class/dmi/id/product_version 2>/dev/null || true) == P86VE ]] ||
    die 'this guarded prototype is restricted to the GIGABYTE AERO 16 YE5 (P86VE)'

devices=()
target=
for device in /sys/bus/hid/devices/0003:1044:7A3A.*; do
    [[ -e $device ]] || continue
    hid_id=$(cat "$device/uevent" 2>/dev/null | sed -n 's/^HID_ID=//p')
    [[ $hid_id == 0003:00001044:00007A3A ]] || die "unexpected HID identity: $device"
    interface=$(cat "$device/../bInterfaceNumber" 2>/dev/null || true)
    [[ $interface =~ ^0[0-9]+$ ]] || die "could not read USB interface number: $device"
    devices+=("$device")
    [[ $interface == 02 ]] && target=$device
done
[[ ${#devices[@]} -gt 0 && -n $target ]] ||
    die 'GIGABYTE 1044:7A3A HID interface 2 was not found'

validate_interfaces() {
    local device driver
    for device in "${devices[@]}"; do
        [[ -e $device ]] || return 1
        driver=$(basename "$(readlink -f "$device/driver" 2>/dev/null || true)")
        [[ $driver == hid-generic ]] || return 1
    done
}
validate_interfaces || die 'every matching HID interface must already be bound to hid-generic'

descriptor=$(sha256sum "$target/report_descriptor" | awk '{print $1}')
case $descriptor in
    "$source_descriptor_sha256") brightness_was_attached=false ;;
    "$brightness_descriptor_sha256") brightness_was_attached=true ;;
    "$prototype_descriptor_sha256") die 'the production identity object is already attached' ;;
    *) die "unexpected interface-2 descriptor hash: $descriptor" ;;
esac
descriptor_size=$(wc -c <"$target/report_descriptor")
[[ $descriptor_size -eq 253 || $descriptor_size -eq 306 ]] ||
    die "unexpected interface-2 descriptor length: $descriptor_size"

loader_dir="$root/target/udev-hid-bpf/target/release"
if command -v udev-hid-bpf >/dev/null 2>&1; then
    loader_path=$(command -v udev-hid-bpf)
else
    [[ -x $loader_dir/udev-hid-bpf ]] ||
        die 'build udev-hid-bpf with tools/brightness-hid-bpf-loader-build.sh'
    loader_path=$loader_dir/udev-hid-bpf
fi

runtime_parent=$(mktemp -d '/var/tmp/aorus-fn-identity-recovery.XXXXXX')
runtime_helper="$runtime_parent/aorus-brightness-hid-bpf"
runtime_restore_helper="$runtime_parent/aorus-brightness-hid-bpf-restore"
runtime_loader="$runtime_parent/udev-hid-bpf"
runtime_object="$runtime_parent/0010-Gigabyte__AERO-16-YE5.bpf.o"
runtime_recovery="$runtime_parent/recover.sh"
watchdog_unit="aorus-fn-identity-recovery-$$"
needs_recovery=false
watchdog_started=false
log_dir=

restore() {
    local status=0
    set +e
    if [[ $needs_recovery == true ]]; then
        "$runtime_helper" recover "$target" || status=$?
    fi
    if [[ $brightness_was_attached == true ]]; then
        "$runtime_restore_helper" test-add "$target" || status=$?
    fi
    validate_interfaces || status=1
    return "$status"
}

cleanup() {
    local status=0
    set +e
    if [[ $watchdog_started == true ]]; then
        systemctl stop "$watchdog_unit.timer" "$watchdog_unit.service" >/dev/null 2>&1 || true
        systemctl reset-failed "$watchdog_unit.service" >/dev/null 2>&1 || true
    fi
    restore || status=$?
    if [[ -n $log_dir ]]; then
        chown -R -- "$desktop_uid:$desktop_gid" "$log_dir" 2>/dev/null || true
    fi
    rm -rf -- "$runtime_parent"
    return "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

install -m 0755 "$loader_path" "$runtime_loader"
# The helper verifies the loaded link by the production object stem. Keep the
# prototype in the watchdog-backed runtime directory, but give this temporary
# copy that expected basename; the installed production object is untouched.
install -m 0644 "$prototype" "$runtime_object"

# The private helper is the existing helper with only its loader path adapted
# for this temporary transaction.
python3 - "$helper" "$runtime_helper" "$runtime_restore_helper" "$runtime_loader" \
    "$prototype_descriptor_sha256" "$brightness_descriptor_sha256" <<'PY'
from pathlib import Path
import sys

source, destination, restore_destination, loader, prototype_hash, brightness_hash = sys.argv[1:]
text = Path(source).read_text()
original_text = text
if f"fixed_descriptor_sha256={prototype_hash}" not in text:
    raise SystemExit("existing helper has an unexpected production descriptor hash")
if f"legacy_brightness_descriptor_sha256={brightness_hash}" not in text:
    raise SystemExit("existing helper has an unexpected legacy descriptor hash")
old_loader = (
    "if [[ -x /usr/local/libexec/aorus-udev-hid-bpf ]]; then\n"
    "  loader=/usr/local/libexec/aorus-udev-hid-bpf"
)
if old_loader not in text:
    raise SystemExit("existing brightness helper has an unexpected loader selection")
text = text.replace(old_loader, f"if true; then\n  loader={loader}", 1)
Path(destination).write_text(text)
restore_text = original_text.replace(old_loader, f"if true; then\n  loader={loader}", 1)
Path(restore_destination).write_text(restore_text)
PY
chmod 0755 "$runtime_helper" "$runtime_restore_helper"

# Create the unconditional systemd recovery before any detach or replacement.
cat >"$runtime_recovery" <<EOF
#!/usr/bin/env bash
set +e
status=0
"$runtime_helper" recover "$target" || status=\$?
if [[ $brightness_was_attached == true ]]; then
    "$runtime_restore_helper" test-add "$target" || status=\$?
fi
exit "\$status"
EOF
chmod 0755 "$runtime_recovery"
systemd-run --quiet --unit "$watchdog_unit" --on-active=60s \
    --timer-property=AccuracySec=1s "$runtime_recovery"
watchdog_started=true

# If the working brightness path is attached, remove only its BPF link and
# wait for the exact source descriptor to return.
needs_recovery=true
if [[ $brightness_was_attached == true ]]; then
    "$runtime_helper" recover "$target" ||
        die 'could not temporarily detach the current brightness-only attachment'
    for _ in {1..40}; do
        descriptor=$(sha256sum "$target/report_descriptor" 2>/dev/null | awk '{print $1}')
        [[ $descriptor == "$source_descriptor_sha256" ]] && break
        sleep 0.05
    done
    [[ $descriptor == "$source_descriptor_sha256" ]] ||
        die 'the original brightness descriptor did not return after recovery'
    validate_interfaces || die 'brightness recovery did not leave every interface on hid-generic'
fi

# Attach the separate prototype object; the production brightness object is
# never replaced, even temporarily.
AORUS_HID_BPF_OBJECT="$runtime_object" "$runtime_helper" test-add "$target"

for _ in {1..40}; do
    descriptor=$(sha256sum "$target/report_descriptor" 2>/dev/null | awk '{print $1}')
    [[ $descriptor == "$prototype_descriptor_sha256" ]] && break
    sleep 0.05
done
[[ $descriptor == "$prototype_descriptor_sha256" ]] ||
    die "prototype descriptor hash did not appear after attachment: $descriptor"
validate_interfaces || die 'prototype attachment changed a HID interface driver'

mapfile -t events < <(python3 - "${devices[@]}" <<'PY'
import pathlib
import sys

events = set()
for device in map(pathlib.Path, sys.argv[1:]):
    events.update(f"/dev/input/{path.name}" for path in device.glob("input/input*/event*"))
print(*sorted(events), sep="\n")
PY
)
mapfile -t identity_events < <(python3 - "$target" <<'PY'
import pathlib
import sys

target = pathlib.Path(sys.argv[1])
word_bits = 64
required = (183, 184, 185, 186, 187, 189, 192)
for path in sorted(target.glob("input/input*/event*")):
    try:
        words = (path.parent / "capabilities/key").read_text().split()
    except OSError:
        continue
    values = [int(word, 16) for word in reversed(words)]
    if all(code // word_bits < len(values)
           and values[code // word_bits] & (1 << (code % word_bits))
           for code in required):
        print(f"/dev/input/{path.name}")
PY
)
(( ${#events[@]} > 0 )) ||
    die 'no evdev nodes were found under the target HID interface'
(( ${#identity_events[@]} > 0 )) ||
    die 'the identity evdev device with all implemented F-key identities did not appear'

printf '\nNative HID-BPF Fn identity prototype is attached to %s.\n' "$target"
printf 'All matching HID interfaces remain on hid-generic; no persistent identity marker changed.\n'
printf 'Capturing %s composite-keyboard event nodes; %s node(s) advertise all implemented F identities.\n' \
    "${#events[@]}" "${#identity_events[@]}"
printf 'The temporary identity node is exclusively grabbed so saved actions do not suspend or alter system state during validation.\n'
printf 'The test checks only one tap per implemented button; it makes no hold/repeat assumptions.\n'
printf 'During the next 45 seconds, tap each in order and release it normally:\n'
printf '  brightness-down, brightness-up, fan, sleep/Zz, Wi-Fi, square-X, AI\n'
printf 'Do not press display/LCD, touchpad-lock, or airplane mode: all remain native and untouched by this test.\n\n'
log_dir=$(mktemp -d '/tmp/aorus-fn-identity-events.XXXXXX')
pids=()
for event in "${events[@]}"; do
    evtest_args=("$event")
    for identity_event in "${identity_events[@]}"; do
        if [[ $event == "$identity_event" ]]; then
            evtest_args=(--grab "$event")
            break
        fi
    done
    timeout 45 evtest "${evtest_args[@]}" >"$log_dir/${event##*/}.log" 2>&1 &
    pids+=("$!")
done
sleep 0.2
for pid in "${pids[@]}"; do
    kill -0 "$pid" 2>/dev/null ||
        die "an evtest capture process exited before validation; event log: $log_dir"
done
set +e
for pid in "${pids[@]}"; do
    wait "$pid"
done
set -e

f13=$(grep -hE 'Event:.*KEY_F13.*value 1' "$log_dir"/*.log | wc -l || true)
f14=$(grep -hE 'Event:.*KEY_F14.*value 1' "$log_dir"/*.log | wc -l || true)
f15=$(grep -hE 'Event:.*KEY_F15.*value 1' "$log_dir"/*.log | wc -l || true)
f16_press=$(grep -hE 'Event:.*KEY_F16.*value 1' "$log_dir"/*.log | wc -l || true)
f16_release=$(grep -hE 'Event:.*KEY_F16.*value 0' "$log_dir"/*.log | wc -l || true)
f17=$(grep -hE 'Event:.*KEY_F17.*value 1' "$log_dir"/*.log | wc -l || true)
f19=$(grep -hE 'Event:.*KEY_F19.*value 1' "$log_dir"/*.log | wc -l || true)
f22=$(grep -hE 'Event:.*KEY_F22.*value 1' "$log_dir"/*.log | wc -l || true)
semantic=$(grep -hE 'Event:.*KEY_(BRIGHTNESSDOWN|BRIGHTNESSUP|POWER|SLEEP|WAKEUP).*value [012]' \
    "$log_dir"/*.log | wc -l || true)
unexpected=$(grep -hE 'Event:.*type 1 \(EV_KEY\).*value [12]' "$log_dir"/*.log |
    grep -vE 'KEY_F(13|14|15|16|17|19|22)' | wc -l || true)
printf 'Native identity tap counts: F13=%s F14=%s F15=%s F16=%s/%s F17=%s F19=%s F22=%s\n' \
    "$f13" "$f14" "$f15" "$f16_press" "$f16_release" "$f17" "$f19" "$f22"
printf 'Original semantic events (brightness/power/sleep)=%s; airplane/display were not inspected or changed.\n' \
    "$semantic"
printf 'Unexpected non-identity key presses from the composite keyboard=%s.\n' "$unexpected"
(( f13 >= 1 && f14 >= 1 && f15 >= 1 && f16_press >= 1 && f16_release >= 1 &&
   f17 >= 1 && f19 >= 1 && f22 >= 1 )) ||
    die "native Fn identity tap validation failed; event log: $log_dir"
(( semantic == 0 )) ||
    die "prototype emitted duplicate original brightness/power/sleep semantics; event log: $log_dir"
(( unexpected == 0 )) ||
    die "a tested button also emitted an unexpected native key sequence; event log: $log_dir"

printf 'Guarded Fn identity prototype passed. It will now be detached and restored.\n'
printf 'No persistent identity loading was enabled; review the event log: %s\n' "$log_dir"
