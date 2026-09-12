# Hardware baseline

This is a dated, read-only engineering snapshot of the verified AERO 16 YE5.
It documents how the controls were identified; it is not a list of required
kernel versions or a guarantee that another laptop is compatible. See the
root [README](../README.md) for supported installation targets.

Collected on 2026-08-26 at 11:46 MDT with read-only commands. No `sudo`, sysfs
write, service restart, or `/etc` modification was performed while collecting
this record.

## Machine

| Item | Value |
| --- | --- |
| Product name | `AERO 16 YE5` |
| Product version | `P86VE` |
| BIOS | `FB0A`, `09/06/2022` |
| OS/kernel | Pop!_OS 24.04; `7.0.11-76070011-generic` |
| Architecture | `x86_64` |
| AORUS sysfs root | `/sys/devices/platform/aorus_laptop` |
| AORUS hwmon identity | `name=aorus_laptop` |
| Current hwmon symlink | `/sys/class/hwmon/hwmon5` (not stable; do not hard-code) |
| Kernel module file | `/lib/modules/7.0.11-76070011-generic/updates/dkms/aorus-laptop.ko.zst` |
| `modinfo` version | `0.01` |
| Existing package/source | `aorus-laptop/0.1.0-git0002d21` via DKMS, as previously recorded |

Commands used:

```sh
cat /sys/class/dmi/id/product_name
cat /sys/class/dmi/id/product_version
cat /sys/class/dmi/id/bios_version
cat /sys/class/dmi/id/bios_date
uname -a
modinfo aorus-laptop
```

## AORUS controls

The following values were read directly from the platform device:

```text
battery_cycle=145
charge_limit=97
charge_mode=0
debug_method=0, 0
fan_curve_index=0
fan_curve_data=0 57
fan_custom_speed=25
fan_mode=2
gpu_boost=0
usb_charge_s3_toggle=0
usb_charge_s4_toggle=0
```

The current firmware profile is therefore `fan_mode=2` (Gaming). The current
selected curve point is index 0, temperature 0°C, raw level 57.

The driver source confirms 15 points and the wire format:

```text
FAN_CURVE_POINTS = 15
fan_curve_data reads: temperature speed
fan_curve_data writes: packed = (raw_speed << 8) | temperature
```

The safe baseline collector did not write `fan_curve_index`. This driver only
exposes the selected point through `fan_curve_data`; reading all 15 points
requires selecting each index, which changes a writable sysfs attribute. The
complete curve must therefore be captured by the daemon only inside an
explicit, guarded hardware test window, with the original index restored.

## Sensors

The AORUS hwmon device was discovered by its `name` file, not by assuming a
numbered hwmon directory:

```text
path=/sys/class/hwmon/hwmon5 name=aorus_laptop
fan1_input=4687
fan2_input=4718
fan3_input=0
fan4_input=0
temp1_input=83000
temp2_input=66000
temp3_input=83000
```

Fan channels 1 and 2 are the useful installed fans on this model. Channels 3
and 4 currently report zero and should be omitted rather than shown as real
fans. EC temperature labels remain source-qualified until runtime correlation
is complete; at this sample, `coretemp` package temperature was 83000
millidegrees Celsius and NVIDIA reported 66°C.

Additional read-only readings:

```text
coretemp temp1_input (Package id 0)=83000
NVIDIA GeForce RTX 3080 Ti Laptop GPU=66°C, 0% utilization, 13.80 W
```

## Power and backlight

```text
Power Profile: Performance
CPU: 9% - 100%, Turbo
Backlight intel_backlight: 4/400 = 1%

/sys/class/backlight/intel_backlight
actual_brightness=4
brightness=4
max_brightness=400
type=raw
bl_power=0
```

The System76 power profile is the authority for CPU/system power policy.

At collection time the Rust D-Bus name
`io.github.aoruslinux.Control1` was not owned, so no Rust daemon status was
available yet.

## Safe re-check commands

These commands are suitable for future read-only comparisons:

```sh
uname -r
cat /sys/class/dmi/id/product_name /sys/class/dmi/id/product_version
cat /sys/class/dmi/id/bios_version /sys/class/dmi/id/bios_date
cat /sys/devices/platform/aorus_laptop/fan_mode
cat /sys/devices/platform/aorus_laptop/fan_curve_index
cat /sys/devices/platform/aorus_laptop/fan_curve_data
for d in /sys/class/hwmon/*; do
  [ "$(cat "$d/name" 2>/dev/null)" = aorus_laptop ] || continue
  cat "$d/temp1_input" "$d/temp2_input" "$d/temp3_input" 2>/dev/null
  cat "$d/fan1_input" "$d/fan2_input" 2>/dev/null
done
system76-power profile
```
