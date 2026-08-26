# AORUS Control implementation TODOs

There are two phases. Phase 1 contains the entire application and every planned feature. Its workstreams are ordered by dependency but are not separate phases or partial releases. Phase 2 contains only the eventual persistent replacement of the Python service and the resulting soak.

Check an item only when its acceptance note is true.

## Phase 1 — complete application

Phase 1 ends with a built, packaged, and hardware-validated daemon, CLI, native UI, fan-curve editor, power controls, diagnostics, secondary controls, and brightness-key fix. The Python service remains the installed profile-sync authority. Rust write tests use explicit, exclusive test windows and restore Python afterward.

### Workstream A — preserve the working baseline

- [ ] Record direct outputs for model/BIOS, module version, AORUS sysfs nodes, hwmon readings, System76 profile, and backlight devices in `docs/hardware-baseline.md`.
- [ ] Export the current 15-point fan curve without modifying it.
- [ ] Save the current Python service unit, script, config, status, and recent journal output.
- [x] Verify the Python watchdog update is installed and logs `Scheduled fan profile watchdog every 60s` (confirmed 2026-08-26).
- [ ] Write and test a Python-service recovery procedure before enabling any Rust writer.

Acceptance: the current working configuration and curve can be identified and restored without relying on memory.

### Workstream B — create the minimal Rust package

- [ ] Initialize one Cargo package with shared library code and `aorusd`, `aorusctl`, and `aorus-control` binaries.
- [ ] Add only the initial dependencies: `zbus`, `serde`, `toml`, and optional `eframe`/`egui_plot` behind a `ui` feature; add another crate only when standard-library/platform support is insufficient.
- [ ] Add `rustfmt`, Clippy, and release-build checks.
- [ ] Add a short README with build prerequisites and the hardware warning.

Acceptance: all three empty/minimal binaries build, and no process needs root merely to open the UI or request status.

### Workstream C — implement read-only hardware discovery

- [ ] Discover `/sys/devices/platform/aorus_laptop` and report missing/permission errors clearly.
- [ ] Discover the AORUS hwmon directory by reading each device's `name`, never by `hwmonN` number.
- [ ] Read all available EC temperatures and fan RPM channels with units.
- [ ] Read fan mode, all 15 curve points, charge mode/limit, GPU boost, battery cycle, and read-only USB states.
- [ ] Read DMI model, BIOS version/date, kernel, and module version for diagnostics.
- [ ] Read the active System76 power profile over D-Bus, with UPower PowerProfiles as a fallback.
- [ ] Correlate EC temperature channels with `coretemp` and NVIDIA readings under idle and load; document labels and uncertainty.
- [ ] Represent optional/missing capabilities explicitly instead of substituting zero.
- [ ] Add fake-sysfs tests for discovery, values, malformed input, disappearing nodes, and permissions.

Acceptance: a read-only `aorusctl status --direct` or test harness matches direct machine readings and performs no writes.

### Workstream D — implement and test the fan-curve model

- [ ] Define `FanMode`, `FanCurve`, and 15 `(temperature, raw_speed)` points.
- [ ] Implement packed data encoding as `(raw_speed << 8) | temperature` and exact decoding.
- [ ] Validate 15 points, 0–100 °C, 0–255 raw speed, and non-decreasing temperatures/speeds.
- [ ] Implement neighbour clamping used by drag and numeric edits.
- [ ] Test minimum/maximum values, equal adjacent values, decreasing values, wrong point counts, and encode/decode round trips.

Acceptance: invalid or partially specified curves cannot reach any hardware-write function.

### Workstream E — implement serialized hardware writes

- [ ] Put every multi-node fan operation behind one daemon-side mutex.
- [ ] Implement fan-profile selection for Normal, Silent, Gaming, and Custom.
- [ ] Port profile reselection behavior that safely toggles through another firmware profile when the cached mode must be resent.
- [ ] Implement full curve snapshot, temporary Gaming selection, 15-point write, exact readback, rollback, and final Custom selection.
- [ ] Leave Gaming active and surface a critical error if both apply and rollback verification fail.
- [ ] Implement validated charging mode/limit writes.
- [ ] Probe this model's valid GPU-boost values before implementing its write path.
- [ ] Keep Auto/Fixed and `fan_custom_speed` out of normal profile workflows.
- [ ] Add fake-sysfs failure injection at each curve index and verify rollback behavior.

Acceptance: tests prove that a partial or unverified curve is never deliberately activated and no normal path writes a fixed fan speed.

### Workstream F — build the daemon and local API

- [ ] Implement `aorusd` on the system D-Bus as `io.github.aoruslinux.Control1`.
- [ ] Implement `GetStatus` and `GetFanCurve` without authorization prompts.
- [ ] Implement the planned mutation methods through the shared validation/write paths.
- [ ] Add polkit authorization using the caller's system-bus identity.
- [ ] Add a system D-Bus policy that exposes only the intended service/API.
- [ ] Add atomic `/etc/aorus-control.toml` loading and updates.
- [ ] Reject a Custom profile mapping until a validated custom curve exists.
- [ ] Log startup state, profile events, repairs, writes, verification failures, and last error without flooding the journal.
- [ ] Add a read-only/shadow mode for migration testing.
- [ ] Add the hardened `aorusd.service` unit and restart behavior.

Acceptance: an ordinary desktop user can read status; unauthorized callers cannot mutate hardware; the UI itself never runs as root.

### Workstream G — port profile synchronization and reliability behavior

- [ ] Subscribe to System76 `PowerProfileSwitch`.
- [ ] Subscribe to UPower PowerProfiles as a fallback and deduplicate matching events.
- [ ] Subscribe to logind resume and reselect after the existing settling delay.
- [ ] Watch meaningful UPower AC/battery changes and reselect after they settle.
- [ ] Add low-frequency mismatch reconciliation without changing the System76 profile.
- [ ] Implement explicit `ReapplyFanProfile` using firmware profiles only.
- [ ] Test daemon startup before/after the driver appears.
- [ ] Test System76 D-Bus restarts and temporary disappearance.
- [ ] Verify failures are retried safely and visible in status/journal.

Acceptance: Rust shadow logs predict the same desired actions as the Python service through profile switches, AC changes, resume, and watchdog checks.

### Workstream H — build the CLI before the GUI

- [ ] Implement `aorusctl status` with temperatures, RPM, profiles, charge state, capabilities, and last error.
- [ ] Implement `aorusctl curve show` with all 15 exact raw points.
- [ ] Implement power-profile and firmware fan-profile selection.
- [ ] Implement `aorusctl fan reapply`.
- [ ] Implement a concise copyable `aorusctl diagnostics` report.
- [ ] Return useful non-zero exit codes and errors for missing daemon, driver, permission, validation, and verification failures.

Acceptance: every hardware and profile action needed by the GUI can first be exercised and recovered from the terminal.

### Workstream I — fix Fn brightness keys

- [ ] Install/use `evtest` or `libinput debug-events` and capture both Fn brightness keys on likely keyboard, Video Bus, and GIGABYTE consumer-control devices.
- [ ] Record standard key events, unknown scan codes, or absence of events in `docs/brightness-debug.md`.
- [ ] If standard events arrive, repair the COSMIC/Pop!_OS shortcut or backlight authorization path.
- [ ] If unknown scan codes arrive, add a DMI/device-specific udev hwdb mapping to `KEY_BRIGHTNESSDOWN` and `KEY_BRIGHTNESSUP`.
- [ ] If no events arrive, trace ACPI/WMI notification delivery and prepare the smallest driver hotkey patch.
- [ ] Verify ten presses in each direction, behavior at min/max, reboot persistence, and suspend/resume.
- [ ] Decide separately whether a standard backlight slider adds value to the app.

Acceptance: both keys adjust `/sys/class/backlight/intel_backlight` reliably while the AORUS app is closed.

### Workstream J — build the native dashboard

- [ ] Open an unprivileged eframe/egui native window and connect to the daemon.
- [ ] Show CPU/GPU/board or clearly labelled EC temperatures.
- [ ] Show Fan 1/Fan 2 RPM and omit unavailable zero-only channels.
- [ ] Show current System76 power and AORUS fan profiles.
- [ ] Add Battery, Balanced, and Performance controls.
- [ ] Add driver/daemon/stale-data/last-error health states.
- [ ] Add a bounded in-memory temperature/RPM history only if live readings alone are insufficient.
- [ ] Keep the UI responsive when D-Bus or sysfs operations time out.

Acceptance: the dashboard accurately follows profile and sensor changes without elevated privileges or unbounded history growth.

### Workstream K — build the fan-curve editor

- [ ] Plot all 15 points with °C on X and raw level/percentage on Y.
- [ ] Add draggable points constrained by range and neighbours.
- [ ] Add keyboard-accessible numeric editors for the selected point.
- [ ] Overlay current temperature markers without confusing RPM and curve-level units.
- [ ] Track dirty state locally; dragging never writes hardware.
- [ ] Add Apply, Discard/Reload, and Reapply actions with clear confirmation/error states.
- [ ] Show write/verify progress and the daemon's rollback result.
- [ ] Add mapping controls so each power profile can select Normal, Silent, Gaming, or a valid Custom curve.
- [ ] Test pointer, keyboard, scaling, and error states.

Acceptance: the user can safely edit, review, apply, verify, discard, and reapply a custom firmware curve without any fixed-speed write.

### Workstream L — add supported secondary controls

- [ ] Add charge mode and a validated 60–100 charge-limit control.
- [ ] Show battery cycle count.
- [ ] Add only the GPU-boost modes confirmed on this laptop, with explanatory text.
- [ ] Show USB S3/S4 charge states as read-only.
- [ ] Show System76 graphics mode/power as read-only first.
- [ ] Document logout/reboot and external-display consequences before allowing graphics-mode changes.
- [ ] Keep `debug_method` inaccessible from the UI.

Acceptance: controls are capability-gated and every write has readback, a visible result, and a safe failure state.

### Workstream M — prove replacement readiness without cutting over

- [ ] Run `aorusd` in read-only/shadow mode while Python remains the sole fan-mode writer.
- [ ] Compare intended Rust actions against Python journal actions during normal use.
- [ ] Add an explicit migration command/script that detects and stops/disables `aorus-power-profile-sync.service` before enabling Rust write ownership.
- [ ] Refuse or prominently fail installation if both services would become writers.
- [ ] Preserve the Python script, service, config, and documented rollback path.
- [ ] Add an exclusive hardware-test procedure that stops Python, starts Rust as the sole writer, and always restores Python afterward.
- [ ] During exclusive test windows, test repeated profile changes, AC transitions, and suspend/resume without enabling a persistent cutover.
- [ ] During exclusive test windows, run monitored CPU/GPU load tests in all intended profiles.
- [ ] During exclusive test windows, run curve apply/readback/restore and forced-failure rollback tests on hardware.
- [ ] During exclusive test windows, verify charging and proven GPU controls, then restore their original state.
- [ ] Verify the test procedure's failure trap restores Gaming or the prior safe firmware profile before restarting Python.
- [ ] Confirm after every exclusive test that Rust write mode is stopped and `aorus-power-profile-sync.service` is active again.

Acceptance: the complete Rust application passes shadow and exclusive hardware tests, concurrent writers are prevented, and Python is still the active installed profile-sync service.

### Workstream N — package the complete application

- [ ] Add install/uninstall scripts or packages for binaries, systemd, D-Bus policy, polkit policy, config, desktop entry, and icon.
- [ ] Install Rust in read-only/shadow mode by default while the Python service owns profile synchronization.
- [ ] Require the separate explicit migration operation before enabling persistent Rust write ownership.
- [ ] Make uninstall preserve user configuration unless explicitly requested.
- [ ] Add license and attribution for the existing AORUS driver behavior being integrated.
- [ ] Document supported hardware as this tested model first; call other models experimental until proven.
- [ ] Publish known limitations, recovery instructions, and diagnostic-report steps.
- [ ] Produce a Phase 1 release candidate only after every Phase 1 workstream and blocker in `PLAN.md` is cleared.

Acceptance: the complete application can be installed, upgraded, tested, and uninstalled without replacing Python, creating competing writers, or erasing user configuration.

## Deferred Phase 2 — persistently replace the Python service

Do not begin this phase until all Phase 1 workstreams pass. This phase changes deployment ownership; it does not add app features.

- [ ] Run the explicit migration operation: stop and disable `aorus-power-profile-sync.service`, then enable persistent Rust write ownership and `aorusd.service`.
- [ ] Verify immediately that `aorusd` is the only fan-control writer.
- [ ] Test repeated profile changes, AC transitions, reboot, and at least ten suspend/resume cycles after cutover.
- [ ] Re-run monitored CPU/GPU load and curve apply/readback/rollback checks in the deployed configuration.
- [ ] Soak for multiple days and review restarts, errors, profile drift, memory, and idle CPU.
- [ ] Compare Rust and Python resident memory, startup time, idle CPU, event latency, and restart behavior before claiming an efficiency improvement.
- [ ] If any cutover gate fails, execute the documented rollback and restore the Python service.
- [ ] If the soak passes, accept Rust as the active service while retaining a documented Python rollback package.
- [ ] Tag the stable release only after the Phase 2 soak passes.

Acceptance: Rust is the sole persistent fan-control service, passes the soak without unexplained drift, and the preserved Python service can still be restored in one documented operation.
