# Fn-input reliability implementation record

## Goal

Make every captured AERO 16 YE5 Fn button reliable across boot, logout,
greeter, desktop changes, suspend/resume, HID reprobe, and changing event
numbers. Every button uses one native HID pipeline: no evdev keymap or hwdb
production path.

## Original problem

The first implementation normalized vendor reports into F13/F14/etc. and then
mutated an ephemeral evdev keymap with `EVIOCSKEYCODE_V2`. That failed after
reprobe and was not a true native path. Generated hwdb rules were also
rejected: hwdb cannot reliably remap this already-normalized HID event stream.
The implementation below replaces both mechanisms with a native HID-BPF
translation and a versioned pinned action map.

## Target architecture

1. Keep exact DMI, VID/PID, interface, source-descriptor, and report checks.
2. HID-BPF translates all captured reports into stable native HID reports.
3. Standard actions use standard HID keyboard/consumer usages: brightness,
   sleep, radio, media, and screenshot. Display and touchpad remain their
   firmware-native interface-0 actions because their duplicate chords cannot
   yet be suppressed safely.
4. AORUS-only actions use stable reserved keyboard usages from the exact
   translated HID interface.
5. `aorusd` consumes only those stable translated AORUS identities; standard
   actions remain in the kernel input stack.
6. Firmware pulse buttons preserve native pulse/repeat semantics. No uinput,
   synthetic input, hidraw listener, or userspace repeat/release loop.
7. The UI updates mappings through D-Bus; the privileged loader updates and
   verifies the complete BPF map with rollback on failure and reports its
   generation.
8. udev manages exact-match HID-BPF attachment. Resume/reprobe repair is a
   safety net that verifies attachment and map generation, not event numbers.

## Implemented design

### 1. Native translated report and map ABI

- Define a version-2 BPF map containing all remappable physical buttons, their
  validated translated-report actions, and a 32-bit generation marker.
- Add standard keyboard/consumer collections and stable reserved keyboard identities.
- Translate brightness, fan, sleep, Wi-Fi, square-X, and AI reports through the
  map; preserve display/touchpad duplicate-chord behavior explicitly.
- Compile the exact report fixup and use the guarded native test for the
  brightness press/hold path; keep the remaining button checks in the live
  acceptance gate because they exercise real firmware and desktop behavior.

### 2. Privileged loader

- Add transactional map update/readback operations, including rollback of all
  entries and generation validation.
- Reject unknown ABI versions, buttons, actions, and incomplete configurations.
- Preserve the prior map if any update fails.
- Remove `EVIOCSKEYCODE_V2`, generated hwdb files, and capability guesses from
  the production path.
- Select the daemon reader by exact HID parent and the translated F13-F22
  capability set, choosing the narrowest matching input node rather than an
  event number.

### 3. Daemon, D-Bus, and UI

- Make `aorusd` consume only stable translated AORUS identities.
- Keep standard actions entirely in the kernel input stack.
- Make mapping writes update the BPF map immediately and verify every entry and
  generation.
- Expose separate attached/map-loaded/map-generation/daemon-reader-ready
  status.
- Keep profile-based fan behavior and never add fixed fan speeds.

### 4. Lifecycle, packaging, and migration

- Install the exact-match udev HID-BPF rule and versioned object/map ABI.
- Repair attachment/map state on boot, resume, and HID reprobe.
- Upgrade an older pinned object/map ABI in place without rebinding unrelated
  keyboard interfaces; no evdev/hwdb migration is needed because those paths
  are no longer part of production.
- Keep rollback at BPF attachment/map level only; no evdev production fallback.
- Never unload or rebind the composite keyboard during normal operation.

### 5. Tests and acceptance

Automated tests cover the map ABI, invalid configurations, install/uninstall,
and Clippy. The guarded native test covers the brightness press/hold path.

Guarded hardware acceptance must cover tap and hold for every button, every
standard action, every fan/power profile, disabled mappings, remapping each
button twice, logout/greeter, KDE, GNOME, suspend/resume, HID reprobe, changing
event numbers, and external keyboard safety.

## Non-goals

No hidraw listener, uinput device, synthetic input, userspace repeat loop,
hwdb remapping, evdev ioctl mapping, broad product match, or fixed fan-speed
control.

## Release status

- [x] Root cause documented: ephemeral evdev mapping is not a reliable primary
  path.
- [x] Invalid hwdb implementation identified and removed from the target design.
- [x] Versioned BPF action map and loader-side updates cover all seven
  interface-2 remappable reports. Standard brightness is native HID and daemon
  actions use stable translated keyboard usages.
- [x] Native standard HID usages for brightness.
- [x] Stable native HID path for daemon actions.
- [x] D-Bus/UI map generation and status.
- [x] Lifecycle upgrade and package cleanup.
- [x] Automated Rust, packaging, udev, systemd, and shell validation.
- [x] Guarded brightness press/hold validation on the native standard HID path.
- [ ] Full live acceptance after reboot, logout/greeter, desktop changes,
  suspend/resume, and HID reprobe.

The remaining item is hardware coverage, not another implementation path. The
installed system should be validated with the exact commands in the main
README and `docs/brightness-debug.md`; no evdev, hwdb, hidraw, or uinput
fallback is planned.
