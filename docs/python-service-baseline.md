# Python profile-sync service baseline

Captured on 2026-08-26 before the Rust daemon existed. This is a comparison baseline, not a claim that Rust will necessarily use fewer resources.

```text
service: aorus-power-profile-sync.service
state: active/running
restart count: 0
process: python3 /usr/local/libexec/aorus-power-profile-sync watch
elapsed at sample: 2557 seconds
systemd MemoryCurrent: 8,871,936 bytes
systemd MemoryPeak: 22,958,080 bytes
ps RSS: 12,764 KiB
ps VSZ: 49,992 KiB
systemd CPUUsageNSec: 650,875,000 ns
```

The current unit's `systemd-analyze security` exposure score is 9.6 (`UNSAFE`), largely because it has no sandboxing directives. Compare the eventual Rust unit with the same command, but do not add hardening that blocks its required system D-Bus and AORUS sysfs access.

After the deferred cutover, sample both implementations over comparable uptime and events. Compare resident memory, peak memory, idle CPU, event response, restarts, and profile drift.

## Recovery snapshot

Rechecked read-only on 2026-08-26 at 12:45 MDT:

```text
ActiveState=active
SubState=running
UnitFileState=enabled
NRestarts=0
MainPID=3136253
MemoryCurrent=8327168
MemoryPeak=22958080
CPUUsageNSec=858362000
```

Installed-file fingerprints:

```text
a294021cfee6970a3a1dde594b000a7573e752d2d6df5bf716b6a1a68d28861d  /usr/local/libexec/aorus-power-profile-sync
9c4b758f7bb833aad6c794fca12dadd8ddde4b489514dab4d3d28400cd56763c  /etc/systemd/system/aorus-power-profile-sync.service
e7cfddc4c15cd0800c21d5d139ab51f28059b873e9351450e327589ee006b8ee  /etc/aorus-power-profile-sync.conf
```

Editable source copies remain in `/home/jordan/src/aorus-power-profile-sync/`.
Exact installed-file copies plus status and recent journal output are preserved
under `baseline/python-service/` in this repository.
The installed service logs `Scheduled fan profile watchdog every 60s` and
`Watching system76-power / PowerProfiles / logind resume / UPower AC-battery
signals` at startup.

Recovery after any temporary Rust write test:

```sh
sudo systemctl stop aorusd-exclusive.service aorusd.service
sudo rm -f /etc/systemd/system/aorusd.service.d/mode.conf
sudo systemctl daemon-reload
sudo systemctl enable --now aorus-power-profile-sync.service
sudo /usr/local/libexec/aorus-power-profile-sync force
systemctl is-enabled aorus-power-profile-sync.service
systemctl is-active aorus-power-profile-sync.service
```

The final two commands must report `enabled` and `active`. The repository's
guarded Phase 1 procedure is `tools/exclusive-hardware-test.sh`; it automates
this restoration and refuses to create a persistent cutover.
