# Changelog

## 0.1.0 — 2026-09-11

Initial release for the tested GIGABYTE AERO 16 YE5 (`P86VE`).

- Native Rust daemon, desktop UI, tray integration, and `aorusctl` CLI.
- Profile-based firmware fan control with validated custom curves.
- System76 power-profile synchronization and transient profile-change OSD.
- System-wide remappable Fn buttons through the exact-model HID-BPF path.
- Temperature, fan-speed, charging, GPU-boost, and hardware diagnostics.
- Optional ambient-light IIO bridge and automatic brightness service.
- Write-enabled installation by default with polkit authorization and rollback.

Hardware writes and native Fn-key translation remain intentionally gated to
the tested AERO 16 YE5 model and its verified report descriptor.
