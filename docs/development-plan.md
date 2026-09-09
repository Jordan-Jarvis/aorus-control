# AORUS Control for Linux — implementation plan

> Historical design record. See [todo.md](todo.md) for validation status and
> the repository README for current installation instructions.

## Goal

Build a small native Rust application for this GIGABYTE AERO 16 YE5 that provides the useful parts of AORUS Control Center on Pop!_OS:

- live CPU, GPU, and board temperatures;
- live fan RPM for both installed fans;
- Battery, Balanced, and Performance power-profile selection;
- profile-based fan control, including reliable reapplication after profile, power, and resume events;
- a 15-point custom fan-curve graph with draggable points;
- charging and supported GPU controls;
- diagnostics that make driver or profile-sync failures visible;
- user-configurable hotkey mappings backed by the desktop's global shortcut system;
- working native Fn buttons, with safe defaults and user-selectable actions for
  brightness down/up, fan, sleep, Wi-Fi, display, Square-X, touchpad lock, and
  AI; the already-native airplane and volume buttons remain Linux-owned;
- ambient-light telemetry through Linux IIO, with an opt-in automatic
  brightness policy where the desktop does not provide one;
- a resident status-area icon with close-to-hide, Open, Quit, and hidden login
  startup behavior.

The application will not set a fixed fan speed during normal operation. Normal, Silent, Gaming, and Custom are firmware profiles. Fixed and Auto modes stay out of the main UI because they rely on `fan_custom_speed` rather than a temperature curve.

## Hardware and software baseline

This plan targets the machine currently being tested:

| Item | Detected value |
| --- | --- |
| Laptop | GIGABYTE AERO 16 YE5 (`P86VE`) |
| OS | Pop!_OS 24.04 LTS |
| Kernel | `7.0.11-76070011-generic` |
| BIOS | `FB0A`, 2022-09-06 |
| Kernel module | `aorus-laptop/0.1.0-git0002d21` via DKMS |
| AORUS sysfs root | `/sys/devices/platform/aorus_laptop` |
| Existing sync service | `aorus-power-profile-sync.service` |
| Power API | `com.system76.PowerDaemon` on the system D-Bus |
| Backlight | `/sys/class/backlight/intel_backlight`, range 0–400 |

The app must discover hwmon and backlight paths by their `name`/device identity. It must not assume that `hwmon5` or any other numbered path remains stable across boots.

### Confirmed AORUS controls

| Control | Interface | Values/notes |
| --- | --- | --- |
| Fan profile | `fan_mode` | 0 Normal, 1 Silent, 2 Gaming, 3 Custom, 4 Auto, 5 Fixed |
| Curve point selector | `fan_curve_index` | writable indices 0–14 |
| Curve point data | `fan_curve_data` | read as `temperature speed`; write `(speed << 8) | temperature` |
| Fixed/auto speed | `fan_custom_speed` | 25–100 in increments of 5; not used by normal profile control |
| Charging mode | `charge_mode` | 0 Normal, 1 Custom |
| Charge limit | `charge_limit` | 60–100; effective in Custom charging mode |
| GPU boost | `gpu_boost` | driver accepts 0–3; supported values must be probed on this model |
| Battery cycles | `battery_cycle` | read-only |
| USB sleep/hibernate charging | `usb_charge_s3_toggle`, `usb_charge_s4_toggle` | read-only in the current driver |

The AORUS hwmon device supplies `temp1_input`, `temp2_input`, `temp3_input`, and `fan1_input` through `fan4_input`. On this laptop, the first two fan channels are the useful fans and channels 3/4 currently report zero. The three EC temperatures will be labelled CPU, GPU, and board only after validating them against `coretemp` and NVIDIA readings; until then the UI should make the EC source explicit.

System76's daemon reports the active power/graphics state, but its charge-threshold call currently returns `Not running System76 firmware with charge threshold support`. Charging therefore uses the confirmed AORUS sysfs controls on this machine; the UI must not present two competing charge backends.

## Product scope

There are exactly two delivery phases:

1. **Phase 1 — complete application:** build, integrate, package, and validate the entire daemon, CLI, native UI, fan-curve editor, power modes, secondary controls, diagnostics, and brightness-key fix. The named activities inside this phase are workstreams, not separate feature phases. The Python service remains installed and is restored as the active authority after any controlled hardware test window.
2. **Phase 2 — Python service replacement:** perform the final persistent cutover from the Python service to the already-complete Rust daemon, then soak and either accept or roll back that cutover.

Nothing except the actual persistent replacement and post-cutover soak is deferred to Phase 2.

### Phase 1 — complete application

The complete Phase 1 application includes:

1. A dashboard with current temperatures, fan RPM, fan profile, power profile, and driver/daemon health.
2. Power-profile controls that call System76's existing API instead of implementing CPU power policy.
3. Default mappings:
   - Performance → Gaming
   - Balanced → Normal
   - Battery → Silent
4. A custom curve editor for all 15 firmware points, with preview, validation, apply, and restore-current actions.
5. Charge mode/limit, battery cycles, and proven GPU-boost choices.
6. A root daemon that owns all Rust-app hardware writes and contains the finished replacement logic, while the Python profile-sync service remains the installed authority until Phase 2.
7. A small CLI for diagnostics, scripted testing, and recovery when the GUI is unavailable.
8. A machine-specific Fn-button fix at the lowest layer that receives each
   proven key event, beginning with the already working brightness pair.
9. A native Hotkeys screen with separate editors for physical laptop Fn
   buttons and conventional global key combinations. Both use a typed action
   allowlist and COSMIC's global shortcut configuration.
10. A standard IIO ambient-light source from the laptop's WMI sensor and an
    opt-in user-session auto-brightness policy if COSMIC still lacks one.
11. A single resident desktop process that starts hidden at login, keeps a
    native status-area item, hides instead of exiting when its window closes,
    and provides explicit Open and Quit actions.

### Deferred until there is a demonstrated need

- named libraries of multiple custom curves;
- fan-speed prediction or automatic tuning;
- per-process performance profiles;
- RGB keyboard control, because the current driver exposes no safe RGB interface;
- writing the read-only USB charging toggles;
- a permanent fixed-speed control in the main UI;
- kernel-driver changes unrelated to brightness-key delivery;
- remote control, HTTP services, or a web UI.

Unsupported controls should appear as unavailable with a reason, not as switches that silently fail.

## Architecture

Use one Cargo package initially, with a shared library and three binaries:

```text
aorus-control/
├── Cargo.toml
├── src/
│   ├── lib.rs                 shared models, validation, sysfs and D-Bus helpers
│   ├── bin/aorusd.rs          privileged system daemon
│   ├── bin/aorusctl.rs        diagnostic/recovery CLI
│   └── bin/aorus-control.rs   native eframe/egui desktop UI
├── packaging/
│   ├── aorusd.service
│   ├── io.github.aoruslinux.Control1.conf
│   └── io.github.aoruslinux.control.policy
└── docs/
    ├── development-plan.md
    └── todo.md
```

Modules should be split into additional files only when their size makes that clearer. Separate crates can be introduced later if independent release cycles or dependency boundaries become a real problem.

### Native UI

Use `eframe/egui` plus `egui_plot`. It creates a normal native Linux window and gives direct support for custom plotting and draggable controls without HTML, JavaScript, a browser runtime, or a local web server. Keep both UI dependencies behind an optional Cargo `ui` feature so daemon/CLI-only builds do not compile the graphics stack.

The UI always runs as the logged-in user. It never runs under `sudo` and never writes sysfs directly. An XDG autostart entry launches it hidden at login. A native StatusNotifierItem reopens the window or quits the resident UI, and a per-user Unix socket keeps launches single-instance.

### Privileged daemon

`aorusd` runs as a root systemd system service and is the only long-running process allowed to mutate AORUS sysfs controls. It will:

- discover and read AORUS sysfs/hwmon nodes;
- expose read-only telemetry and capability information over the system D-Bus;
- validate and serialize hardware mutations;
- call `com.system76.PowerDaemon` for CPU/system power-profile changes;
- watch System76, UPower, and logind signals;
- reselect the mapped fan profile after resume and relevant AC/battery changes;
- reconcile a visible fan-mode mismatch on a low-frequency watchdog;
- persist only user-approved mappings and custom curves;
- log useful failures to the system journal.

No thermal control loop belongs in the daemon. Firmware remains responsible for adapting fan speed to temperature according to the selected firmware profile or custom curve.

### CLI

`aorusctl` uses the same D-Bus API as the UI. Its first commands should be:

```text
aorusctl status
aorusctl curve show
aorusctl curve capture
aorusctl profile performance|balanced|battery
aorusctl fan normal|silent|gaming|custom
aorusctl fan reapply
aorusctl diagnostics
```

Mutation commands go through the daemon and the same authorization and validation paths as GUI actions. A narrowly scoped offline/root recovery command can be added only if D-Bus recovery proves insufficient.

## D-Bus contract

Use a versioned system-bus API:

- bus name: `io.github.aoruslinux.Control1`
- object path: `/io/github/aoruslinux/Control1`
- interface: `io.github.aoruslinux.Control1`

Initial methods:

| Method | Purpose | Authorization |
| --- | --- | --- |
| `GetStatus() -> a{sv}` | temperatures, RPM, profiles, charge state, capabilities and last error | none |
| `GetFanCurve() -> a(yy)` | return the 15 `(temperature, raw_speed)` points | none |
| `CaptureFanCurve() -> a(yy)` | validate and store the current firmware curve without changing the active fan profile | polkit |
| `SetPowerProfile(s)` | call System76 Battery/Balanced/Performance method | polkit |
| `SetFanMode(y)` | select an allowed firmware profile | polkit |
| `ReapplyFanProfile()` | safely reselect the mapped profile | polkit |
| `SetFanCurve(a(yy))` | validate, write, verify, persist and select Custom | polkit |
| `SetProfileMappings(a{sy})` | update power-profile-to-fan-profile mappings | polkit |
| `SetChargeMode(y)` | change Normal/Custom charging mode | polkit |
| `SetChargeLimit(y)` | set a validated 60–100 limit | polkit |
| `SetGpuBoost(y)` | set a model-supported boost mode | polkit |

The flexible status dictionary is acceptable for the local, versioned v1 API; every key and unit must be documented before the UI depends on it. Temperatures use millidegrees Celsius internally, RPM is an integer, and curve speed remains the firmware's raw 0–255 value. The UI may display curve speed as a percentage, but it must preserve the exact raw value on round trips.

One polkit action, `io.github.aoruslinux.control.modify`, authorizes mutations for an active local user. Read-only status remains available without prompting. The daemon must authorize the D-Bus caller's unique bus name, not a PID supplied by the caller.

## Fan-curve model and safety

A valid curve has exactly 15 points. For each point:

- temperature is 0–100 °C;
- raw fan level is 0–255;
- temperatures never decrease from left to right;
- fan levels never decrease from left to right.

Dragging a point clamps it between its neighbours so the graph cannot enter an invalid state. Keyboard adjustment and accessible numeric controls must be available in addition to pointer dragging.

Applying a curve is a guarded operation:

1. Read and retain all 15 current points and the current fan profile.
2. Validate the entire proposed curve before touching hardware.
3. Temporarily select Gaming while rewriting a curve that may currently be active; this is a firmware profile, not a hard-coded speed.
4. Hold one daemon-side hardware mutex while selecting each index and writing its packed value.
5. Read all 15 points back and compare them exactly.
6. On success, persist the curve atomically and select Custom.
7. On failure, attempt to restore the previous points. If restoration cannot be verified, leave the machine in Gaming and report a prominent error rather than activating a partial curve.

The UI has separate Edit, Apply, and Discard/Reload states. Opening or dragging the graph must never write to hardware. A Reset action first reloads the firmware's current curve; factory defaults cannot be claimed unless the driver or firmware provides a trustworthy source for them.

Only one custom curve is needed for v1. Any power profile may map to Custom after a curve has been successfully stored. Mapping to Custom without a valid stored curve must be rejected.

## Power-profile synchronization

System76 remains the authority for CPU/system power modes. `aorusd` maps its profile to an AORUS firmware fan profile and preserves the reliability behavior learned from the Python service:

- subscribe to `PowerProfileSwitch` from `com.system76.PowerDaemon`;
- subscribe to UPower PowerProfiles as a fallback and deduplicate equivalent events;
- reapply after logind's resume event, after a short settling delay;
- reapply after meaningful UPower AC/battery transitions;
- periodically repair cases where the desired and reported fan modes differ;
- offer explicit `reapply` for the driver's known stale cached-mode condition.

Reselection may briefly use another safe firmware profile before restoring the requested profile so the out-of-tree driver sends the command again. It never uses a fixed fan speed.

## UI layout

### Dashboard

- large CPU and GPU temperature readings plus board temperature;
- Fan 1 and Fan 2 RPM;
- current System76 power profile and current AORUS fan profile;
- three power-profile buttons;
- health banner for missing driver, daemon, sensors, stale data, or last failed write;
- optional short rolling temperature/RPM history kept only in memory.

### Fans

- Normal, Silent, Gaming, and Custom profile selector;
- profile mapping controls for Battery, Balanced, and Performance;
- 15-point curve plot with temperature on X and raw level/percentage on Y;
- current temperature markers over the curve;
- selected-point numeric editors;
- Apply, Discard/Reload, and Reapply Current Profile actions.

RPM is displayed beside the curve but not plotted as if it were the same unit as the 0–255 firmware fan level.

### Power and battery

- System76 power profile;
- charge mode and charge limit;
- battery cycle count;
- graphics mode/power as read-only information first, with switching added only after its logout/reboot behavior is verified.

### GPU and hardware

- GPU boost values that have been proven safe on this model;
- read-only USB sleep/hibernate charging state;
- driver, BIOS, model, sysfs-node, and sensor diagnostics;
- copyable diagnostic report with no secrets.

## Brightness-key workstream

Fn brightness handling should work independently of whether the AORUS UI is open.

1. Capture key presses with `evtest` or `libinput debug-events` on the internal keyboard, Video Bus devices, and GIGABYTE HID consumer-control device.
2. If `KEY_BRIGHTNESSDOWN`/`KEY_BRIGHTNESSUP` already arrive, fix the Pop!_OS/COSMIC shortcut or backlight authorization path.
3. If Linux receives unknown scan codes, add a machine-specific udev hwdb mapping to standard brightness key codes.
4. If no standard input event arrives, add the smallest native kernel HID/input support that does not replace or unbind the physical keyboard driver.
5. Verify both keys across reboot and suspend/resume, including native hold/repeat behavior and brightness limits.

The confirmed source is USB HID `1044:7a3a`, interface 2. Its vendor report
`04 00 00 7d` means brightness down and `04 00 00 7e` means brightness up on
this DMI-matched laptop. A product-wide HID special driver cannot safely claim
only interface 2: its ID makes `hid-generic` relinquish all four interfaces
before a probe callback can reject the other three. The discarded test
disabled the internal keyboard and was removed. The safe intended fix is a
native kernel HID/input integration, preferably HID-BPF or an equivalent
narrowly scoped kernel path, that keeps `hid-generic` attached to the
composite device. It must gate attachment by the target DMI identity, USB
interface 2, and the expected report-descriptor shape (including report ID
`0x04`), then translate only the exact captured reports into normal
`KEY_BRIGHTNESSDOWN` and `KEY_BRIGHTNESSUP` events.

The implementation must not match the product-wide USB ID and reject sibling
interfaces in `probe`: that ordering already caused the keyboard failure. It
must preserve unrelated reports and remain inactive when the DMI or descriptor
does not match. The standard HID/input stack must own the resulting evdev
device; no hidraw-to-uinput userspace re-emitter is part of the design.

The raw capture proves one report per brightness press and no release report.
The HID-BPF descriptor therefore adds relative Consumer brightness fields and
translates each exact vendor report to one native HID press/release pulse,
without an invented timeout or userspace repeat loop. Hold behavior and repeat
cadence must be tested without claiming success before live validation.

The app may later expose a brightness slider through the standard
backlight/logind interface. Brightness keys must remain independent of the
app and must work while it is closed.

## Ambient-light and automatic-brightness workstream

The WMI `f7` notifications are ambient-light samples, not hotkeys. Expose the
24-bit little-endian lux value as a standard IIO illuminance channel using a
DMI-gated kernel driver. This keeps sensor acquisition in the kernel interface
expected by `iio-sensor-proxy` and other desktop components.

The installed COSMIC settings daemon currently has manual brightness methods
but no ambient-light/SensorProxy consumer. If that remains true after the IIO
device is validated, add the smallest opt-in user-session policy. It must use
hysteresis and settling delays, pause after manual brightness changes, clamp to
a user-configurable minimum, and call the desktop's normal brightness API. It
must not run in root `aorusd`, write a fixed brightness continuously, or hide
the standard IIO sensor behind an application-only API.

## Hotkey mapping

Hotkeys configured in the app must continue working after its window closes.
On this COSMIC system, the UI should manage narrowly identified entries in the
user's native `com.system76.CosmicSettings.Shortcuts` configuration instead of
opening `/dev/input`, adding a second key-grabber, or running commands from the
root daemon. Existing non-AORUS custom shortcuts must be preserved semantically.

The first supported actions are opening AORUS Control, selecting Battery,
Balanced, or Performance, selecting/reapplying a firmware fan profile, and
the desktop's standard brightness/volume/media actions. Arbitrary shell
commands are out of scope because a fixed action list covers the requested
control-center behavior without creating a command-execution interface.

The Hotkeys screen must capture one combination at a time, detect conflicts,
allow clearing/restoring defaults, show when the current desktop backend is
unsupported, and verify that the compositor accepted each change. The native
kernel brightness path must emit ordinary brightness key events independently
of the application and its global shortcut configuration.

## Physical Fn-button remapping

Physical laptop buttons and conventional global shortcuts are different
models. A conventional shortcut maps an action to a user-entered key
combination. An Fn mapping maps one fixed physical button identity to one
allowlisted action. They may share the action catalogue and COSMIC serializer,
but they must not share persistence records or UI rows.

The stable physical IDs are:

```text
brightness-down  brightness-up  fan  sleep  wifi  display
square-x  touchpad-lock  ai
```

The 2026-08-28 read-only capture identified these reports:

| Physical button | Confirmed report | Status |
| --- | --- | --- |
| Brightness down | interface 2: `04 00 00 7d` | production F13 identity; persistent evdev and COSMIC action validated 2026-09-02 |
| Brightness up | interface 2: `04 00 00 7e` | production F14 identity; persistent evdev and COSMIC action validated 2026-09-02 |
| Fan | interface 2: `04 00 00 84` | production F15 identity; live validation pending |
| Sleep / Zz | interface 2: `02 02` press, `02 00` release | production F16 identity; live validation pending |
| Wi-Fi | interface 2: `04 00 00 7c` | production F17 identity; live validation pending |
| Display / LCD | interface 0: Right-Super+P keyboard sequence | native chord used directly |
| Square-X | interface 2: `04 00 00 80` | production F19 identity; live validation pending |
| Touchpad lock | interface 2: `04 00 00 81` plus interface-0 sequence | native Super+Ctrl+F24 chord used; interface-2 report ignored |
| AI | interface 2: `04 00 00 88` | production F22 identity; live validation pending |

Airplane mode emits HID Wireless Radio Control report `07 01` and already
works through Linux. It is intentionally not a managed physical button, has
no AORUS-owned F21 shortcut, and remains untouched like the working volume
keys. Airplane toggle remains an assignable action for managed buttons.

The WMI `f7......` buffers are ambient-light samples and must not be reused as
hotkey codes. Event-device numbers are also unstable and must not be stored.

### Native input path

The target path is:

```text
firmware vendor report
  -> DMI/interface/descriptor-gated HID-BPF translation
  -> hid-generic
  -> distinct standard evdev identity key
  -> COSMIC global shortcut dispatcher
  -> selected typed action
```

This remains native Linux input: there is no root hidraw listener, uinput
device, application key grabber, userspace repeat loop, or command wrapper.
Every composite interface stays attached to `hid-generic`, unrelated reports
pass through byte-for-byte, and persistent loading retains the existing
automatic recovery guard.

The production HID-BPF object maps the seven interface-2 reports to
modifierless F13, F14, F15, F16, F17, F19, and F22 identities. Display and
touchpad lock retain their already-distinct native interface-0 chords. The
identity range is an implementation detail and is never user-editable.

Enabling native Fn support first installs and reads back the default or saved
COSMIC mappings, then attaches and verifies the production HID-BPF object.
Failure removes the enable marker, detaches the object, and restores the exact
original descriptor. Upgrading a prior brightness-only object is handled as a
guarded in-place migration; one report never emits both its old semantic event
and its new identity.

### Capture and event semantics

Add one generalized read-only capture that prompts for a tap and a two-second
hold for each button and records the exact HID interface, report length and
bytes, evdev/libinput events, repeat cadence, and relevant before/after state.
Capture sleep under a logind sleep inhibitor. Query WMI/ACPI only when HID and
evdev are silent. After every guarded translation test, verify that all four
composite interfaces remain on `hid-generic`.

- A proven press/release pair uses ordinary down/up semantics.
- A proven single report is tap-only.
- Repeated reports without a release may use relative native pulses only for
  actions safe to repeat, such as brightness or volume.
- Suspend, profile cycling, radio/display/touchpad toggles, and app launch stay
  unavailable for a repeating report until safe edge semantics are proven.
- Never infer a key solely from its printed icon, invent a release timeout, or
  hard-code an unobserved report.

### Action catalogue and defaults

The UI offers a typed enum, grouped as System, Power/Fans, Media, and
Application. Initial candidates are Disabled; brightness down/up; Battery,
Balanced, Performance, and cycle power profile; Normal, Silent, Gaming,
Custom, and reapply fan profile; suspend; Wi-Fi, display, touchpad, and
airplane toggles; screenshot; volume down/up/mute; play/pause; and open AORUS
Control. An action is selectable only after its COSMIC or typed `aorusctl`/D-Bus
backend is proven. Arbitrary commands, arbitrary report bytes/keycodes, and
fixed fan speed are never options.

Proposed defaults, to be verified during capture, are:

| Button | Default action |
| --- | --- |
| Brightness down/up | System brightness down/up |
| Fan | Cycle System76 power profile and therefore its mapped firmware fan profile |
| Sleep / Zz | Suspend |
| Wi-Fi | Wi-Fi toggle |
| Display / LCD | Display toggle |
| Square-X | Open AORUS Control |
| Touchpad lock | Touchpad toggle |
| AI | Open AORUS Control until a native Linux AI action exists |

Cycling the System76 profile follows the configured firmware mapping regardless
of which profile-sync service is authoritative. Direct fan-profile choices use
the Rust daemon's write-enabled API and always select firmware profiles, never
a fixed speed.

### Persistence, conflicts, UI, and recovery

Store the button-to-action choices in a versioned user-owned
`$XDG_CONFIG_HOME/aorus-control/fn-buttons.toml`, written atomically with mode
`0600`. Compiled defaults fill missing entries in memory; explicit Reset one
and Reset all actions apply the current defaults without overwriting existing
choices during upgrades. Unknown schema versions fail closed without changing
the file.

COSMIC entries are derived, AORUS-owned records named by physical button, for
example `AORUS Control: fn-button:fan`. Saving validates the complete proposed
set, checks COSMIC default/custom conflicts, preserves unrelated shortcuts,
writes and reads back the COSMIC configuration, and only then commits the
user mapping file. Failure restores the previous AORUS-owned entries. Multiple
buttons may intentionally choose one action, but two physical identities may
not claim one trigger key. External edits to an AORUS-owned entry require an
explicit reload rather than silent overwrite.

The Hotkeys page has two sections. **Laptop Fn buttons** shows one responsive
row/card per physical button with detection state, native trigger evidence,
Action combo box, default, capability reason, immediately saved Reset, and
Reset all. Selector changes save immediately so controls cannot be separated
from an off-screen Apply button. **Global keyboard shortcuts** retains the existing combination editor.
At narrow widths, controls stack rather than forming a wide matrix. Disabled
and Not captured are explicit states, not silent fallbacks.

The UI is only the editor. Mappings work with the window closed because COSMIC
dispatches standard evdev triggers to system actions or fixed typed
`aorusctl` commands. Add matching recovery commands:

```text
aorusctl fn list
aorusctl fn get BUTTON
aorusctl fn set BUTTON ACTION
aorusctl fn reset [BUTTON|all]
```

These commands run as the desktop user and never accept shell text. The root
daemon does not own per-user shortcut configuration.

## Configuration

Store daemon-managed configuration in `/etc/aorus-control/config.toml`, owned by root and written atomically. The initial schema is deliberately small:

```toml
version = 1

[profile_mappings]
performance = "gaming"
balanced = "normal"
battery = "silent"

# Added only after the user explicitly applies a validated curve:
# [custom_curve]
# points = [ ... exactly 15 [temperature, raw_speed] pairs ... ]
```

Delays, sysfs locations, and fan-mode numeric constants stay in code unless testing shows a machine-specific adjustment is necessary. The daemon should preserve unknown configuration keys only if forward compatibility actually requires it; malformed safety-critical values cause a clear startup/config error and are not guessed.

## Deferred Phase 2 — replacing the Python service

Rust is a good fit, but replacement is justified by single-writer ownership and measured reliability—not by assuming a Rust process is automatically better.

Phase 1 must leave the Rust daemon, migration tooling, rollback instructions, and all application features complete. It may use short, controlled hardware-test windows in which the Python service is stopped, Rust is tested as the only writer, and Python is restarted afterward. Phase 1 never leaves Rust persistently installed as the profile-sync authority.

Before Phase 2 starts, Phase 1 must have passed these readiness gates:

1. **Read-only parity:** Rust reports the same power profile, fan mode, sensors, curve, and charging state as direct sysfs/System76 queries.
2. **Shadow reliability:** `aorusd` observes profile, AC, resume, and watchdog events without writing while the Python service remains authoritative. Its logged intended actions must match the Python service.
3. **Exclusive hardware validation:** with Python stopped temporarily, Rust passes profile switching, curve apply/rollback, charging/GPU controls, and recovery tests; Python is then restored.
4. **Cutover readiness:** installation detects the existing service, prevents concurrent writers, and has a tested rollback operation.

Phase 2 then performs only the deployment transition:

1. Stop and disable `aorus-power-profile-sync.service`.
2. Enable Rust mutations and `aorusd.service`, ensuring Rust is the sole fan-control writer.
3. Pass profile switching, AC transitions, repeated suspend/resume, reboot, curve apply/rollback, and sustained CPU/GPU load tests.
4. Soak for multiple days, then either accept the cutover or restore Python.

The cutover installer must detect an active Python service and refuse to enable Rust write ownership until the operator explicitly migrates. Rollback restores the Python service and its existing configuration; it must not delete user curves or logs.

After cutover, compare resident memory, startup time, idle CPU use, event latency, and failure/restart counts. The expected result is one smaller operational surface and no Python/GLib runtime dependency for this function.

## Verification and release gates

### Automated checks

- unit tests for fan modes, packed curve encoding, all validation boundaries, and monotonic constraints;
- fake-sysfs tests for discovery, read/write errors, complete curve round trips, rollback, and missing capabilities;
- config parse/validation/atomic-write tests;
- D-Bus authorization tests for read-only and mutating methods;
- UI model tests for point clamping and dirty/apply/discard state;
- formatting, Clippy with warnings denied in CI, and release builds for all binaries.

### On-hardware checks

- status values agree with sysfs, `sensors`, System76, and NVIDIA tools;
- all three power profiles map correctly at least ten times each;
- unplug/replug AC retains the selected mapping;
- ten suspend/resume cycles restore the correct profile;
- profile reapply ramps fans through firmware control without setting fixed speed;
- curve write/readback is exact and a forced write failure exercises rollback;
- fans respond to a safe test curve under monitored load, then return to the original curve;
- charge limit is verified without repeatedly changing it;
- GPU boost is exposed only after each supported value is identified;
- Fn brightness keys pass before/after reboot and resume;
- every Phase 1 write test runs with Python stopped and ends by restoring Python as the active service;
- after the Phase 2 cutover, the daemon survives a multi-day soak with no unexplained restarts or profile drift.

Phase 1 completion is blocked if any planned app feature is missing, a partial curve can be activated, two services can concurrently own fan-mode writes during testing, the UI requires root, or a profile action silently falls back to fixed fan speed. Final release is additionally blocked until the separate Phase 2 cutover and soak succeed.
