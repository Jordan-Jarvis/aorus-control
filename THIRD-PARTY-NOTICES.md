# Third-party and separately licensed components

The Rust application and its Rust support code are licensed under the MIT
License in [`LICENSE`](LICENSE).

The following native kernel components are separately licensed under the GNU
General Public License. Their SPDX headers are authoritative:

- `brightness/hid-bpf/0010-Gigabyte__AERO-16-YE5.bpf.c` — GPL-2.0-only
- `brightness/als/aorus-als.c` — GPL-2.0-or-later
- `brightness/aorus-hotkey-trace.c` — GPL-2.0-or-later
- `drivers/aorus-laptop-dkms/aorus-laptop.c` — GPL-2.0-or-later (vendored from
  `tangalbert919/gigabyte-laptop-wmi`)

The GPL-2.0 text is included in [`COPYING.GPL-2.0`](COPYING.GPL-2.0).
Components marked “or-later” may also be used under a later GPL version.

The build fetches the pinned `udev-hid-bpf` loader from its upstream project at
<https://gitlab.freedesktop.org/libevdev/udev-hid-bpf>. That project is GPL-2.0-only and its
license text is included in its source checkout and release source archive.
This repository does not redistribute that generated dependency; the build
script pins the commit used for reproducible builds.
