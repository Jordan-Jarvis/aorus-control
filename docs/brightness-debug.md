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
cd /path/to/aorus-control
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

### This machine's first interactive result

The correct 2026-08-26 capture contained no brightness, unknown-key, or scan
event from either physical brightness-key press. The only emitted event was
`MSC_SCAN 70028` followed by `KEY_ENTER`, caused by releasing the Enter key
used to start the capture. The `KEY_BRIGHTNESSUP`/`KEY_BRIGHTNESSDOWN` lines in
the original printed summary were device capability declarations, not events.

The firmware does expose the unclaimed WMI event GUID
`ABBC0F72-8EA1-11D1-00A0-C90629100000`, which the installed `aorus_laptop`
source already names but does not handle. Run the temporary read-only tracer
to capture the real firmware payload before adding a key mapping:

```sh
sudo ./tools/brightness-wmi-capture.sh
```

This builds and loads a temporary event-logging module, asks for one press in
each direction, saves the kernel messages, and unloads the module. It does not
write the backlight, EC, WMI methods, or persistent module configuration.

The resulting buffers started with `f7` and changed repeatedly while a hand
was near the keyboard. Existing Gigabyte firmware disassembly identifies this
payload as ambient-light data (`F7`, followed by three lux bytes), not a
hotkey. No brightness mapping may be inferred from those values.

The internal keyboard also exposes vendor-defined raw HID reports which the
generic HID input mapping may be ignoring. The next read-only capture records
those reports without detaching the keyboard driver:

```sh
sudo ./tools/brightness-hid-capture.sh
```

It provides separate timed windows for brightness-down and brightness-up so
the two raw reports can be distinguished without inventing a mapping.

### Confirmed raw HID brightness reports

The labeled capture on 2026-08-26 identified one report per physical key:

| Action | HID device | Report |
| --- | --- | --- |
| Fn+F3 / brightness down | GIGABYTE `1044:7a3a`, USB interface 2 | `04 00 00 7d` |
| Fn+F4 / brightness up | GIGABYTE `1044:7a3a`, USB interface 2 | `04 00 00 7e` |

The report descriptor for that interface declares report ID `0x04` as three
vendor-defined bytes. The reports occurred in their respective labeled
windows, while the earlier simultaneous evtest capture showed that
`hid-generic` did not turn them into input events. This is sufficient to map
only those reports to standard `KEY_BRIGHTNESSDOWN` and
`KEY_BRIGHTNESSUP` events. It is not a reason to map other `0x04` reports or
to add a privileged userspace key listener. These captures define the native
translation requirements; they do not validate a kernel implementation.

### Full Fn-button capture (2026-08-28)

The read-only capture in `/tmp/aorus-fn-buttons.zzVrap` kept all four
`1044:7a3a` interfaces on `hid-generic` and established these additional
reports:

| Button | Source | Captured report or sequence |
| --- | --- | --- |
| Fan | interface 2 | `04 00 00 84` |
| Sleep / Zz | interface 2 | `02 02` press, `02 00` release |
| Wi-Fi | interface 2 | `04 00 00 7c` |
| Display / LCD | interface 0 | Right-Super+P keyboard press/release sequence |
| Square-X | interface 2 | `04 00 00 80` |
| Touchpad lock | interfaces 2 and 0 | `04 00 00 81` plus a keyboard sequence |
| Airplane mode | interface 2 | `07 01` (HID Wireless Radio Control) |
| AI | interface 2 | `04 00 00 88` |

The physical airplane button already works through Linux and is intentionally
unmanaged, like the working physical volume keys. Report `07 01` must remain
unchanged and F21 is not reserved. The typed airplane-toggle action remains
available for other managed buttons.

The capture's evtest portion was empty because that script version looked for
`/dev/eventN` instead of `/dev/input/eventN`; its ancestor walk also read
write-only sysfs `uevent` controls, producing harmless permission errors. The
fixed tool uses exact discovered HID paths and includes only the nine managed
buttons. Display and the interface-0 half of touchpad still require separate
validation and must not be guessed into the interface-2 BPF program.

### Retired HID-driver test and native replacement

The first implementation registered a special HID driver with the product-wide
`1044:7a3a` ID and rejected the wrong interfaces in `probe`. That is too late:
`hid-generic` checks special-driver ID tables before probe and relinquished all
four same-ID interfaces. A live test on 2026-08-26 left them unbound and
disabled the internal keyboard. The module and every HID bind/unbind test were
removed.

No hidraw-to-uinput replacement is endorsed. That approach does not provide
the requested native key-hold semantics and is not the production design.

The intended fix is a narrowly DMI-, USB-interface-, and descriptor-gated
kernel HID/input path. HID-BPF is the preferred mechanism because it can leave
`hid-generic` attached while fixing the report descriptor and translating
reports before the normal HID input mapper. A purpose-built kernel HID driver
is an alternative only if it claims/handles the composite device without
leaving sibling interfaces unbound. A product-wide `1044:7a3a` match that
rejects interfaces in `probe` is explicitly unsafe and must not return.

For interface 2, the path validates the exact descriptor, report IDs, lengths,
and captured bytes before translating seven managed buttons to otherwise
unused native F-key identities. Unrelated reports remain unchanged, and all
four composite interfaces remain bound to `hid-generic`. Single-report buttons
use relative fields for native pulses; sleep uses its captured press/release
pair. No release timeout or userspace repeat loop is introduced.

The 2026-08-27 recovery verified interfaces 0 through 3 on `hid-generic` and
that the retired module was no longer loaded. This is recovery evidence only,
not validation of the native replacement.

### Production Fn identities

The capture-proven interface-2 reports now become `F13`, `F14`, `F15`,
`F16`, `F17`, `F19`, and `F22`. Display and touchpad lock retain their exact
native interface-0 chords (`Super+P` and `Super+Ctrl+F24`); translating their
simultaneous vendor reports would duplicate one physical press. Airplane mode
remains untouched. Standard XKB exposes `F13`–`F17` to COSMIC as `XF86Tools`
and `XF86Launch5`–`XF86Launch8`; generated shortcuts must use those post-XKB
keysyms rather than the evdev names.

The guarded `tools/fn-identity-hid-bpf-test.sh` test schedules recovery before
attaching, checks every implemented identity, rejects duplicate semantic
events, and verifies every composite interface remains on `hid-generic`.
Persistent activation is performed only after the user's COSMIC mappings are
saved, through the app or `aorusctl fn enable`.

The source descriptor SHA-256 is
`8c466c33cedbb3be04738089da3c09d4319443a793b462319708a8f8364be17a`; the
production fixed descriptor is
`7c69146eea1d52d72015cdcc225e462e7110d4e5f8b26142986231c24a4f8271`.

The WMI `f7...` buffers are a separate ambient-light stream. Their three bytes
form a little-endian lux sample; they must not be interpreted as hotkeys.

### Ambient light and automatic brightness

This installation currently has no IIO light device, `iio-sensor-proxy` is
not installed, and the installed COSMIC settings daemon exposes manual
brightness methods but no SensorProxy/ambient-light API. The correct kernel
hook is therefore a DMI-gated IIO illuminance device fed by the WMI `f7`
notifications. That makes the sensor available to standard Linux consumers
without putting a brightness policy loop in the root AORUS daemon.

Exposing the IIO sensor alone does not enable automatic brightness in this
COSMIC release. A desktop/session policy must consume it, apply hysteresis,
and respect manual overrides before automatic adjustment can be enabled.

No brightness slider is planned in AORUS Control. Brightness should continue
to work through the standard kernel backlight and desktop controls while the
app is closed; duplicating that control would hide rather than fix the Fn-key
delivery problem. Global hotkey mappings remain a separate desktop shortcut
configuration feature and must not require the AORUS app or a privileged raw
input listener.

## Native-path test gate

Before loading any native HID support on the laptop:

1. Verify the DMI, USB interface number, report descriptor length/hash, and
   report ID/length match the documented target.
2. Use an external keyboard and schedule an unconditional rollback watchdog
   before loading the object. The watchdog must unload the test support,
   restore `hid-generic` bindings, and report failure if any interface is
   missing or the internal keyboard stops responding.
3. Confirm that a failed load cannot leave interfaces 0, 1, or 3 unbound and
   that unrelated HID reports still reach their original input devices.
4. Only after those checks, test down/up presses, held-key native repetition,
   repeat cadence, brightness limits, reboot, and suspend/resume.

The guarded test passed on 2026-08-27: taps and holds produced 10 native
`KEY_BRIGHTNESSDOWN` and 13 native `KEY_BRIGHTNESSUP` events, and all four HID
interfaces remained on `hid-generic`. COSMIC currently launches one asynchronous
D-Bus command per pulse; closely spaced calls race and can collapse into one
brightness step even though evdev received every repeat. This is desktop action
handling, not userspace HID emulation. Brightness limits, persistent loading,
reboot, and suspend/resume still require validation.
