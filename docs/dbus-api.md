# Local D-Bus API v1

This is the integration contract between `aorusd`, `aorusctl`, and the native UI.

- Bus: system
- Destination/interface: `io.github.aoruslinux.Control1`
- Path: `/io/github/aoruslinux/Control1`
- Default daemon mode: `shadow` (read-only while the Python service is authoritative)

## Methods

| Member | Input | Output |
| --- | --- | --- |
| `GetStatus` | none | `a{sv}` |
| `GetFanCurve` | none | `a(yy)` containing exactly 15 `(temperature, raw_speed)` pairs when available |
| `SetPowerProfile` | `s` (`performance`, `balanced`, or `battery`) | none |
| `SetFanMode` | `y` (0 Normal, 1 Silent, 2 Gaming, 3 Custom) | none |
| `ReapplyFanProfile` | none | none |
| `SetFanCurve` | `a(yy)` | none |
| `SetProfileMappings` | `a{sy}` | none |
| `SetChargeMode` | `y` | none |
| `SetChargeLimit` | `y` | none |
| `SetGpuBoost` | `y` | none |

Every mutating method fails while the daemon is in shadow mode. Write mode also requires polkit action `io.github.aoruslinux.control.modify` and is never enabled by the normal Phase 1 installer.

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
| `custom_curve_available` | `b` | complete valid curve can be read |
| `last_error` | `s` | last actionable daemon error; omitted when clear |

All readers ignore unknown keys. RPM and raw fan-curve level are different units and must never be interchanged.

