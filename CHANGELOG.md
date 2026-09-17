# Changelog

## 0.1.3 — 2026-09-17

- Accept firmware fan curves whose raw fan level briefly dips, as exposed by
  the verified AERO 16 YE5 at point 13.
- Keep temperature points ordered while preserving the firmware's actual fan
  levels in the UI and curve editor.

## 0.1.2 — 2026-09-17

- Install the bundled `aorus_laptop` DKMS driver as part of the main Debian
  package installation.
- Declare DKMS, build tools, and generic kernel headers as package
  dependencies so the post-install driver setup does not invoke `apt` inside
  an active package transaction.

## 0.1.1 — 2026-09-14

First public artifact release.

- Hardened native Fn-key attachment across boot, suspend, and HID reprobe.
- Added bundled WMI and ambient-light driver installation helpers.
- Added shared custom-command Fn actions and release-ready Debian packages.

## 0.1.0 — 2026-09-11

Initial release for the tested GIGABYTE AERO 16 YE5 (`P86VE`).

- Native Rust daemon, desktop UI, tray integration, and `aorusctl` CLI.
- Profile-based firmware fan control with validated custom curves.
- System76 power-profile synchronization and transient profile-change OSD.
- System-wide remappable Fn buttons through the exact-model HID-BPF path.
- Temperature, fan-speed, charging, GPU-boost, and hardware diagnostics.
- Optional ambient-light IIO bridge and automatic brightness service.
- Write-enabled installation by default with polkit authorization and rollback.
- Ubuntu/Pop!_OS 24.04 x86_64 `.deb` packages, optional ALS/DKMS package, and tagged GitHub release automation.

Hardware writes and native Fn-key translation remain intentionally gated to
the tested AERO 16 YE5 model and its verified report descriptor.
