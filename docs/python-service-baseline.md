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

