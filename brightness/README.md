# AERO 16 YE5 native Fn-key HID path

This implementation is deliberately restricted to the captured machine:

- DMI: `GIGABYTE`, `AERO 16 YE5`, `P86VE`;
- USB HID: `1044:7a3a`;
- USB interface 2 only; and
- the exact 253-byte source report descriptor with SHA-256
  `8c466c33cedbb3be04738089da3c09d4319443a793b462319708a8f8364be17a`.

Do **not** load the retired `aorus-brightness` HID module. A product-wide HID
driver match makes `hid-generic` relinquish every interface of the composite
keyboard before `probe` can reject the wrong interfaces, disabling the
internal keyboard.

If a retired test left an interface unbound, recover it with an external
keyboard:

```sh
sudo rmmod aorus_brightness 2>/dev/null || true
sudo bash -c 'for d in /sys/bus/hid/devices/0003:1044:7A3A.*; do
  [ -e "$d" ] || continue
  [ -L "$d/driver" ] || printf "%s\n" "${d##*/}" > /sys/bus/hid/drivers/hid-generic/bind
done'
```

## Production implementation

The production HID-BPF program leaves all four interfaces on `hid-generic`.
It translates only the capture-proven interface-2 reports below into ordinary,
modifierless keyboard identities consumed by the user's COSMIC shortcuts:

| Firmware report | Native identity | Physical button |
| --- | --- | --- |
| `04 00 00 7d` | `F13` | brightness down |
| `04 00 00 7e` | `F14` | brightness up |
| `04 00 00 84` | `F15` | fan |
| `02 02` / `02 00` | `F16` press/release | sleep/Zz |
| `04 00 00 7c` | `F17` | Wi-Fi |
| `04 00 00 80` | `F19` | Square-X |
| `04 00 00 88` | `F22` | AI |

Display already emits the distinct native `Super+P` interface-0 sequence.
Touchpad lock already emits `Super+Ctrl+F24` on interface 0; its simultaneous
interface-2 `04 00 00 81` report is deliberately not translated, preventing a
single press from dispatching twice. Airplane mode remains Linux-owned and
unchanged. Unrelated reports pass through byte-for-byte. There is no hidraw
listener, uinput device, userspace repeat loop, or replacement HID driver.

The single-report buttons use relative HID fields, producing native pulses
without inventing release timing. Sleep uses its captured absolute
press/release pair.

Build the program and upstream loader with:

```sh
sudo apt-get install -y libudev-dev libelf-dev
./tools/brightness-hid-bpf-loader-build.sh
./tools/brightness-hid-bpf-build.sh target/aorus-brightness.bpf.o
```

The guarded test attaches temporarily, schedules automatic recovery, and
verifies the implemented identities while every interface remains on
`hid-generic`:

```sh
sudo ./tools/fn-identity-hid-bpf-test.sh --confirm-external-keyboard
```

After installation, enable persistently from **Hotkeys → Laptop Fn buttons**
or as the regular desktop user:

```sh
aorusctl fn enable
```

That command commits the COSMIC mappings before asking the privileged daemon
to create `/etc/aorus-control/brightness-hid-bpf.enabled` and attach HID-BPF.
The udev rule restores the attachment after reboot/replug. Disable it with
`aorusctl fn disable`.

The production fixed descriptor SHA-256 is
`7c69146eea1d52d72015cdcc225e462e7110d4e5f8b26142986231c24a4f8271`.
The helper also recognizes the retired brightness-only descriptor
`637f4bd5f31d413593ab1095e97f0567b4456c04e78bdb1268fc641e3e9e1a48`
solely to detach it safely during upgrade.
