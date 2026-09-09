# AORUS Control for Linux

Native fan, power, temperature, charging, and Fn-key controls for the
GIGABYTE AERO 16 YE5 (`P86VE`). The application consists of a small privileged
Rust daemon, a native Rust desktop UI, and a CLI.

> [!WARNING]
> Hardware writes and the HID-BPF Fn-key fix are intentionally restricted to
> the tested `GIGABYTE AERO 16 YE5 / P86VE`. Do not remove the model,
> interface, or report-descriptor checks to make another laptop appear
> supported.

## Features

- CPU/GPU temperature and dual-fan RPM telemetry
- Firmware Normal, Silent, Gaming, and Custom fan profiles
- Interactive 15-point temperature/fan-curve editor with validation and rollback
- Pop!_OS/System76 power-profile synchronization
- Battery charge mode and charge-limit controls
- Capability-gated GPU boost and USB charging controls
- Native, remappable laptop Fn buttons through HID-BPF, evdev, XKB, and COSMIC
- Optional ambient-light integration and automatic brightness
- StatusNotifierItem tray icon; closing the window keeps the UI resident
- CLI access through `aorusctl`

Fan control is profile-based. Normal operation never writes a fixed fan speed
or `fan_custom_speed`; custom curves are validated and read back by `aorusd`.

## Architecture and safety

| Component | Responsibility |
| --- | --- |
| `aorusd` | Sole privileged hardware writer and system D-Bus service |
| `aorus-control` | Unprivileged native UI and tray process |
| `aorusctl` | Unprivileged CLI client |
| `aorus-auto-brightness` | Optional per-user automatic-brightness policy |

`aorusd` is write-enabled by default and is the sole hardware-control daemon
installed by this project. Hardware mutations are serialized, validated,
authorized through polkit, and restricted to the exact supported model.

The Fn-key implementation is also native: firmware report → exact-model
HID-BPF fixup → `hid-generic` → evdev → XKB → COSMIC. It does not use a
`hidraw` listener, `uinput`, synthetic input, polling, or a userspace repeat
loop. See [brightness/README.md](brightness/README.md) for the hardware gates
and report map.

## Requirements

- GIGABYTE AERO 16 YE5 (`P86VE`)
- Linux with systemd, D-Bus, polkit, udev, and the `aorus_laptop` driver from
  [gigabyte-laptop-wmi](https://github.com/tangalbert919/gigabyte-laptop-wmi)
- Pop!_OS 24.04 with COSMIC for the tested power-profile and global-shortcut
  integration
- Rust stable with Edition 2024 support
- A C toolchain, Clang, pkg-config, libbpf, libudev, libelf, and matching
  kernel headers for the native Fn-key and ambient-light modules

On Ubuntu/Pop!_OS, the development packages are typically:

```sh
sudo apt-get install -y \
  build-essential clang git libbpf-dev libelf-dev libudev-dev pkg-config \
  "linux-headers-$(uname -r)"
```

## Build and install

```sh
git clone https://github.com/Jordan-Jarvis/aorus-control.git
cd aorus-control

cargo build --release --locked

# Build the pinned upstream HID-BPF loader and this laptop's gated BPF object.
./tools/brightness-hid-bpf-loader-build.sh

sudo ./install.sh
```

The installer places binaries under `/usr/local`, installs systemd, D-Bus,
polkit, udev, desktop, icon, and XDG-autostart files, and preserves an existing
`/etc/aorus-control/config.toml`. It enables and starts the write-enabled
`aorusd` service and never invokes `sudo` itself. Use
`DESTDIR=/path/to/staging ./install.sh` to inspect a package staging tree
without changing the live system.

Launch **AORUS Control** from the application menu. Native Fn-key support can
then be enabled under **Hotkeys → Laptop Fn buttons**, or from the desktop user
account with:

```sh
aorusctl fn enable
```

Fn-button action changes save immediately and continue working when the UI is
hidden or fully exited. Airplane mode and the physical volume buttons remain
Linux-owned and are not remapped.

## CLI

Run `aorusctl --help` for the complete command list. Common commands are:

```sh
aorusctl status
aorusctl diagnostics
aorusctl profile performance
aorusctl fan gaming
aorusctl fan reapply
aorusctl curve show
aorusctl mappings get
aorusctl fn list
aorusctl fn set brightness-down disabled
```

The CLI never writes sysfs directly. Mutating operations use the daemon's
typed D-Bus API and polkit authorization.

## Configuration

| Path | Purpose |
| --- | --- |
| `/etc/aorus-control/config.toml` | System profile and fan mappings |
| `~/.config/aorus-control/fn-buttons.toml` | Per-user physical Fn actions |
| `~/.config/aorus-control/auto-brightness.toml` | Optional brightness policy |
| `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom` | Narrowly owned COSMIC shortcut records |

Hardware paths are discovered by device identity; unstable `hwmonN` and
`eventN` numbers are never persisted.

## Uninstall

```sh
sudo ./uninstall.sh
```

Uninstall preserves `/etc/aorus-control/config.toml` and
`/var/lib/aorus-control`.

## Development

Run the complete non-destructive check suite with:

```sh
./tools/check.sh
```

It runs formatting, tests, Clippy, release builds, packaging validation, udev
validation, and an ambient-light module build check. Fn-key hardware
diagnostics are documented in [docs/brightness-debug.md](docs/brightness-debug.md).

Additional references:

- [D-Bus API](docs/dbus-api.md)
- [Hardware baseline](docs/hardware-baseline.md)
- [Native Fn-key implementation](brightness/README.md)

## License

[MIT](LICENSE)
