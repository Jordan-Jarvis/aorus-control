# Fn-key diagnostics

Use this document only when the installed Fn path is not working or when
collecting compatibility results. It is intentionally more technical than the
root README and the capture commands can expose hardware identifiers; review
their output before sharing it.

The tested machine is a GIGABYTE AERO 16 YE5 (`P86VE`) with a composite
`1044:7a3a` USB-HID keyboard. The brightness buttons originate on interface 2
as vendor reports:

| Button | Report |
| --- | --- |
| Brightness down | `04 00 00 7d` |
| Brightness up | `04 00 00 7e` |

The other captured interface-2 reports are fan `04 00 00 84`, sleep `02 02` /
`02 00`, Wi-Fi `04 00 00 7c`, Square-X `04 00 00 80`, and AI `04 00 00 88`.
Display/LCD and touchpad lock use firmware-native interface-0 keyboard
chords. Airplane mode and the physical volume buttons already work through
Linux and are intentionally untouched.

## Production path

The exact-model HID-BPF program fixes only interface 2's report descriptor and
translates those captured reports before `hid-generic` maps them to Linux
input. Standard actions become standard HID brightness, media, radio, sleep,
or screenshot usages. AORUS power/fan actions become reserved F13–F23
identities consumed by the system `aorusd` daemon. The version-2 pinned action
map is updated transactionally and supports disabled mappings without synthetic
input.

This is a kernel input path. It does not use a hidraw listener, uinput,
evdev keymap, hwdb remapping, replacement HID driver, or userspace repeat
loop. All four composite keyboard interfaces remain on `hid-generic`.

The source descriptor hash is
`8c466c33cedbb3be04738089da3c09d4319443a793b462319708a8f8364be17a`; the
current fixed descriptor hash is
`b171e2725b8413d9d10e78709c7f3b36e6e72ce8d304307de1ed25ad4aa44908`.

## Read-only capture

To collect new evidence without changing keyboard behavior:

```sh
sudo ./tools/brightness-capture.sh --duration 30
sudo ./tools/fn-buttons-capture.sh --all
```

The capture bundles may contain hardware identifiers and raw input reports;
review them before sharing. Do not infer a new mapping from a capability line
alone—the button must produce a report during its labeled capture window.

## Guarded native test

Connect an external keyboard and run:

```sh
sudo ./tools/brightness-hid-bpf-test.sh --confirm-external-keyboard
```

The test has an automatic recovery watchdog, validates the native brightness
down/up press and hold path, and checks that all composite interfaces remain
on `hid-generic`. Run it before enabling a custom brightness mapping, or reset
the brightness buttons to their defaults first.

After installation, inspect the persistent path with:

```sh
aorusctl fn status
aorusctl status
```

The daemon repairs the attachment and action map after startup, suspend/resume,
and HID reprobe. If `native_fn_keys_attached` or
`native_fn_keys_reader_ready` becomes false, collect:

```sh
journalctl -u aorusd.service -n 100 --no-pager
```

and the output of `aorusctl fn status` before changing any kernel input state.
