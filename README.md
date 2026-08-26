# AORUS Control for Linux

Native Linux controls for supported GIGABYTE AERO/AORUS laptops. The first
target is the tested GIGABYTE AERO 16 YE5 (`P86VE`) on Pop!_OS 24.04.

The project is intentionally profile-based. Normal, Silent, Gaming, and
Custom are firmware fan profiles; the normal workflow does not set a fixed
fan speed or write `fan_custom_speed`. Custom curves contain the firmware's 15
temperature/raw-level points and are applied only through the daemon's
validation, readback, and rollback path.

## Components

- `aorusd` — privileged system daemon. It owns hardware writes and exposes the
  documented system D-Bus API.
- `aorusctl` — small dependency-minimal CLI client.
- `aorus-control` — unprivileged native UI (when the `ui` feature is built).

The API is documented in [docs/dbus-api.md](docs/dbus-api.md). The package
installer runs `aorusd` in `shadow` mode by default, where it reports state and
can ask System76 to change the CPU power policy, but never writes AORUS sysfs.
Until an exclusive test window has stored a validated curve, the curve editor
is unavailable because reading every firmware point requires selector writes.
The existing
`aorus-power-profile-sync.service`
remains the profile-sync writer during Phase 1. It is never stopped or
disabled by `install.sh`.

## Build and install

Build on the target machine with Rust/Cargo installed:

```text
cargo build --release
```

The package installer must be run by an operator with root privileges; it does
not invoke `sudo` itself:

```text
./install.sh
```

It installs the binaries, systemd/D-Bus/polkit integration, desktop entry,
configuration template, and explicit migration/rollback helpers. Existing
`/etc/aorus-control/config.toml` is preserved. The daemon uses
`ConfigurationDirectory=aorus-control`; the normal shadow unit does not
conflict with or stop the Python profile-sync service. To create a package
staging tree
without touching the live system, use `DESTDIR=/path/to/staging ./install.sh`
after building; the live systemd service is not enabled for a staged install.

## CLI

```text
aorusctl status
aorusctl curve show
aorusctl curve apply TEMP:RAW_SPEED ... # exactly 15 points
aorusctl profile performance|balanced|battery
aorusctl fan normal|silent|gaming|custom
aorusctl fan reapply
aorusctl mappings get
aorusctl mappings set PROFILE=FAN_PROFILE [...]
aorusctl charge mode 0|1|normal|custom
aorusctl charge limit 60-100
aorusctl gpu boost VALUE
aorusctl diagnostics
```

Read-only commands work for ordinary users when the daemon is running.
Mutating commands go through `aorusd` and require its polkit action. System76
power-profile changes work in shadow mode; direct AORUS hardware changes do
not. `aorusctl` returns non-zero errors for an
unavailable daemon, authorization failure, invalid value, or hardware/API
failure.

## Ownership and rollback

Do not run the migration helper until the complete Phase 1 application has
passed shadow and exclusive hardware tests. It is deliberately not called by
the installer:

```text
sudo /usr/local/libexec/aorus-control-migrate-to-rust --confirm-rust-write
```

That operation stops and disables the Python service only after checking that
`aorusd` is healthy and reporting shadow mode. If the cutover fails, it
restores Python automatically where possible. The manual rollback is:

```text
sudo /usr/local/libexec/aorus-control-rollback-to-python --confirm-python
```

Before the persistent cutover, use the temporary exclusive hardware test
window to validate Rust as the sole writer while automatically restoring the
Python service afterward. The installer places the helper at
`/usr/local/libexec/aorus-control-exclusive-hardware-test`; its procedure and
test guidance are documented in
[docs/exclusive-hardware-test.md](docs/exclusive-hardware-test.md):

```text
sudo /usr/local/libexec/aorus-control-exclusive-hardware-test --confirm-exclusive
```

Uninstall first requires rollback if Rust write mode is active. Uninstall
preserves `/etc/aorus-control/config.toml` and `/var/lib/aorus-control`.

## Hardware and limitations

The AORUS driver exposes EC temperatures, fan RPM, charging controls, GPU
boost, and a 15-point fan curve through sysfs. Hardware paths are discovered
by device name at runtime; numbered `hwmonN` paths are not stable. The tested
machine's read-only baseline is in
[docs/hardware-baseline.md](docs/hardware-baseline.md).

Unsupported or unverified controls must be shown as unavailable. RGB control,
read-only USB charging toggles, fixed-speed control in the main UI, and
unverified graphics-mode switching are intentionally outside the normal
workflow.
