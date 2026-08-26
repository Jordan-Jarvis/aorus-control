# Brightness-key diagnostic slice

This machine has a working kernel backlight interface, but the Fn brightness
event path has not been observed yet. No udev hwdb candidate is included: a
machine-specific mapping without an actual captured scan value would be an
unsafe guess.

## Observed state

Read-only inspection on 2026-08-26 found:

| Area | Evidence |
| --- | --- |
| DMI | `GIGABYTE AERO 16 YE5`, product version `P86VE`, BIOS `FB0A` dated `09/06/2022` |
| Kernel | `7.0.11-76070011-generic` |
| Backlight | `/sys/class/backlight/intel_backlight`, raw scale, current/actual `4`, maximum `400`, `bl_power=0` |
| Kernel modules | `aorus_laptop`, `system76_acpi`, `video`, and `wmi` are loaded |
| Likely input sources | `event5` AT Translated Set 2 keyboard; `event8` and `event9` ACPI Video Bus; GIGABYTE USB-HID `event10`, `event12` System Control, `event13` Consumer Control, and `event15` keyboard |
| Capture tools | `evtest` and `libinput` were not installed during inspection |

The GIGABYTE HID device exposes several input interfaces, so looking only at
the ordinary keyboard interface is not sufficient. External Logitech and
Microsoft input devices were present but are not laptop Fn-key candidates.
The loaded `video`/WMI modules and the `intel_backlight` node prove that the
backlight plumbing exists; they do not prove which device emits the Fn keys.

## Capture procedure

The helper is [tools/brightness-capture.sh](../tools/brightness-capture.sh).
It only reads state and writes a local bundle; it does not write brightness,
sysfs, `/etc`, udev rules, or driver settings. It dynamically identifies the
likely devices above, records each with `evtest` and `libinput` when available,
and saves DMI, backlight, ACPI/WMI paths, udev properties, permissions, and
input capabilities.

Install the diagnostic tools using the normal Pop!_OS package mechanism if
they are missing (`evtest` and `libinput-tools` provide the commands). Then
run:

```sh
cd /home/jordan/src/aorus-control
./tools/brightness-capture.sh --duration 30
```

The helper waits for Enter so all event readers are ready. During the stated
capture window, press exactly once each:

1. Fn + brightness down
2. Fn + brightness up

Do not press other keys. The helper prints the output directory when done.
If the normal user cannot read `/dev/input/event*`, rerun the same helper
manually with the required local privilege; the helper itself never invokes
`sudo`.

For a non-interactive run, use `--no-prompt`, but that mode is useful only for
coordinated capture by another process. It cannot identify a brightness key
without the two real key presses.

## Decision tree

Inspect the `events/*.evtest.log` and `events/*.libinput.log` files in the
bundle. The helper also prints a search summary, but an empty search is
inconclusive unless both keys were actually pressed.

### 1. Standard key events

If a likely device reports `KEY_BRIGHTNESSUP` and/or `KEY_BRIGHTNESSDOWN`, the
kernel and input mapping already produce standard Linux key events. Do not add
a hwdb mapping. Continue at the desktop/compositor layer: check whether the
session consumes the events and whether its backlight service can reach
`intel_backlight`. A duplicate report from multiple likely devices should be
treated as one physical key path, not as a reason to install multiple fixes.

### 2. Unknown key or scan event

If the logs contain `MSC_SCAN`, `KEY_UNKNOWN`, or `KEY_RESERVED` without a
standard brightness key name, preserve the exact event lines and the matching
device metadata. Do not infer or substitute a scan value.

The next step is a machine-specific udev hwdb investigation using the exact
device identity from `input-devices.tsv`/the per-device metadata. Only after
that evidence is reviewed should a candidate such as
`brightness/99-aorus-brightness.hwdb` be authored. It must be left
uninstalled until `udevadm test`/reload validation and a reboot or device
reprobe test confirm the mapping. A hwdb rule must match this laptop's
GIGABYTE/i8042 or HID identity narrowly; never match every generic keyboard.

### 3. No event

If both real key presses produce no key, scan, or relevant input event on all
selected devices, the problem is below the desktop binding layer. Check the
kernel log while repeating the capture and inspect ACPI/WMI hotkey support:

```sh
journalctl -k -f
```

In another terminal, run the capture helper again and perform the two presses.
If the journal shows an ACPI/WMI notification but no input event, the next
work is an `aorus_laptop`/ACPI-WMI driver investigation. If it shows nothing,
compare the behavior in firmware/Windows and inspect the DSDT/SSDT hotkey
methods before changing a driver. Do not write a udev rule for a key that
never reaches the input subsystem.

## What counts as proof

The current static evidence proves only that `/sys/class/backlight/intel_backlight`
exists and that there are several plausible event sources. It does not prove a
safe keycode, scan value, hwdb match, or driver fix. The exact interactive
capture above is therefore required before adding anything under `brightness/`.

The diagnostic bundle can contain hardware identifiers and raw input events;
review it before sharing it outside the machine.

No brightness slider is planned in AORUS Control. Brightness should continue
to work through the standard kernel backlight and desktop controls while the
app is closed; duplicating that control would hide rather than fix the Fn-key
delivery problem.
