# Local D-Bus API v1

This reference is for UI, CLI, and desktop integration developers. It is not
needed for normal installation or use; see the root [README](../README.md).

This is the integration contract between `aorusd`, `aorusctl`, and the native UI.

- Bus: system
- Destination/interface: `io.github.aoruslinux.Control1`
- Path: `/io/github/aoruslinux/Control1`
- Default daemon mode: `write-enabled`

## Methods

| Member | Input | Output |
| --- | --- | --- |
| `GetStatus` | none | `a{sv}` |
| `GetProfileMappings` | none | `a{sy}` mapping normalized power-profile names to fan-mode values |
| `GetFnButtonMappings` | none | `a{ss}` mapping stable physical-button IDs to action IDs |
| `GetFanCurve` | none | `a(yy)` containing exactly 15 `(temperature, raw_speed)` pairs when available |
| `CaptureFanCurve` | none | captures, validates, stores, and returns the current 15 firmware points without selecting Custom or changing the active fan profile |
| `SetPowerProfile` | `s` (`performance`, `balanced`, or `battery`) | none |
| `SetFanMode` | `y` (0 Normal, 1 Silent, 2 Gaming, 3 Custom) | none |
| `ReapplyFanProfile` | none | none |
| `SetFanCurve` | `a(yy)` | none |
| `SetProfileMappings` | `a{sy}` | none |
| `SetChargeMode` | `y` | none |
| `SetChargeLimit` | `y` | none |
| `SetGpuBoost` | `y` | none |
| `SetNativeFnKeysEnabled` | `b` | enables/disables the exact-model HID-BPF translation and its persistent udev marker |
| `SetFnButtonMappings` | `a{ss}` | validates, stores, and immediately applies the native HID-BPF action map |

Selecting or reapplying Custom always rewrites and verifies the stored 15-point
curve before activating Custom. The daemon never merely selects Custom after a
firmware reset or an unverified rollback. An unverified rollback disables the
stored Custom curve and resets Custom mappings to conservative firmware
profiles.

Physical Fn mappings are stored in `/etc/aorus-control/fn-buttons.toml`.
Seven capture-proven vendor reports are translated to native HID usages and
mapped to standard Linux input actions or reserved F13–F22 identities consumed
by `aorusd`. The daemon reapplies both the HID-BPF attachment and action map after
resume, device reprobe, and watchdog-detected loss. Display and touchpad lock
retain their firmware-native actions and reject non-default mappings.

## Signals

| Member | Payload | Meaning |
| --- | --- | --- |
| `OpenRequested` | none | an Fn button mapped to `open-app` was pressed; running desktop UI instances show and focus their window |
| `ProfileChanged` | `(ss)` | the daemon applied a power/fan profile; the first field is the normalized power profile when available, and the second is the selected fan profile |

Every mutation requires polkit action
`io.github.aoruslinux.control.modify`. The daemon is write-enabled by default;
an explicit `aorusd --shadow` launch keeps telemetry and System76 power-profile
requests available while disabling direct AORUS hardware writes.

## Status keys

Keys may be added compatibly. Missing/unsupported readings are omitted rather than represented as zero.

| Key | Variant value | Unit/meaning |
| --- | --- | --- |
| `daemon_mode` | `s` | `shadow` or `write-enabled` |
| `driver_available` | `b` | AORUS platform controls discovered |
| `power_profile` | `s` | normalized System76 profile |
| `fan_mode` | `y` | firmware mode 0–5 |
| `temp1_millicelsius`..`temp3_millicelsius` | `i` | EC temperature |
| `fan1_rpm`..`fan4_rpm` | `u` | measured RPM; unsupported channels are omitted |
| `charge_mode` | `y` | AORUS charging mode |
| `charge_limit_percent` | `y` | 60–100 |
| `battery_cycles` | `u` | read-only EC count |
| `gpu_boost` | `y` | current driver value |
| `usb_charge_s3` / `usb_charge_s4` | `b` | read-only capability state |
| `graphics_mode` | `s` | System76 graphics mode when available |
| `graphics_power` | `b` | System76 discrete graphics power when available |
| `custom_curve_available` | `b` | a validated custom curve is stored and may be selected/mapped |
| `native_fn_keys_supported` | `b` | the exact DMI/HID-gated native translation is installed for this laptop |
| `native_fn_keys_enabled` | `b` | persistent loading is enabled |
| `native_fn_keys_active` | `b` | the production translation is attached and the native HID-BPF action map matches the stored configuration |
| `native_fn_keys_map_loaded` | `b` | the pinned native HID-BPF action map is present |
| `native_fn_keys_attached` | `b` | the exact production HID-BPF object and action map are attached |
| `native_fn_keys_map_generation` | `u` | monotonically advancing action-map generation, omitted when the map is unavailable |
| `native_fn_keys_reader_ready` | `b` | a stable translated private-action input node is present for daemon-owned actions |
| `product_name` / `product_version` | `s` | DMI product identity |
| `bios_version` / `bios_date` | `s` | DMI firmware identity |
| `kernel_release` | `s` | running kernel release |
| `driver_module_version` | `s` | loaded `aorus_laptop` module version, when exported |
| `platform_path` / `hwmon_path` | `s` | dynamically discovered AORUS sysfs paths |
| `last_error` | `s` | last actionable daemon error; omitted when clear |
| `cap_fan_modes` | `ay` | fan-mode values supported by this model; values use the same firmware numbering as `fan_mode` |
| `cap_fan_curve_points` | `y` | number of writable fan-curve points, when supported; zero means unavailable |
| `cap_charge_mode` | `b` | charging-mode control is available |
| `cap_charge_limit` | `b` | charging-limit control is available |
| `cap_usb_charge_s3` / `cap_usb_charge_s4` | `b` | corresponding USB charging control is available |
| `cap_gpu_boost_values` | `ay` | verified GPU boost values accepted by this model; empty means unavailable/unverified |

All readers ignore unknown keys. RPM and raw fan-curve level are different units and must never be interchanged.

## Fan-curve read side effect

`GetFanCurve` is logically read-only from the API's point of view, but the
current driver exposes curve points through the writable `fan_curve_index`
selector. The daemon temporarily selects each index to read its point and
restores the previous selector. Consequently, a curve read can fail and can
briefly change that selector; callers should not poll it as telemetry. Curve
reads and writes are serialized by the daemon. Strict shadow mode never writes
the selector: it returns the stored validated curve when one exists, otherwise
`GetFanCurve` reports that the curve is unavailable.
