# Exclusive Phase 1 hardware test

This procedure temporarily makes the Rust daemon the sole AORUS writer. It is
not the persistent Phase 2 migration. The installed Python service must be
enabled and active before the test and is restored by an exit/signal/error
trap.

Build and install the Phase 1 application in shadow mode first. Then open a
window with:

```sh
cd /home/jordan/src/aorus-control
sudo ./tools/exclusive-hardware-test.sh --confirm-exclusive
```

Use another terminal for the planned `aorusctl` profile, curve, charging, and
load checks. Press Enter in the first terminal to close the window. A
non-interactive command can instead follow `--`, for example:

```sh
sudo ./tools/exclusive-hardware-test.sh --confirm-exclusive -- \
  /usr/local/bin/aorusctl fan reapply
```

The script refuses to start unless Python is active, the Rust service reports
shadow mode, and no persistent write-mode drop-in exists. During cleanup it
asks the Rust daemon to restore the prior firmware profile. If the daemon is
unavailable, it selects the Gaming firmware profile directly—never a fixed fan
speed—before restarting Python and forcing its current profile mapping.

After every window, verify:

```sh
systemctl is-active aorus-power-profile-sync.service
systemctl is-active aorusd.service
/usr/local/bin/aorusctl status | grep daemon_mode
systemctl is-active aorusd-exclusive.service || true
```

Expected results are `active`, `active`, `daemon_mode=shadow`, and
inactive/not-found respectively. Do not continue testing if Python was not
restored.

Suspend/resume and sustained CPU/GPU load tests require live temperature and
RPM monitoring and cannot be safely automated by this helper. Record commands,
temperatures, fan RPM, journal output, and the restored state for each test.
