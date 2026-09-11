# AERO 16 ambient-light IIO bridge

This is a deliberately narrow, out-of-tree kernel module for the tested
machine only:

```text
sys_vendor       GIGABYTE
product_name     AERO 16 YE5
product_version  P86VE
```

It listens to WMI GUID
`ABBC0F72-8EA1-11D1-00A0-C90629100000`. Only an ACPI buffer of exactly four
bytes with prefix `0xf7` is accepted. Bytes 1..3 are decoded little-endian as
the lux value. Other object types, lengths, and prefixes are rejected and
rate-limited in the kernel log.

The module exposes one standard IIO `IIO_LIGHT` processed channel:

```text
/sys/bus/iio/devices/iio:device*/in_illuminance_input
```

That is the interface recognized by `iio-sensor-proxy`'s standard udev rules;
there is no custom root brightness loop here. `iio-sensor-proxy` can poll the
cached value from its normal user-space polling path and expose it to a
desktop that supports `net.hadess.SensorProxy`.

## Install

From the repository root:

```sh
sudo apt-get install dkms "linux-headers-$(uname -r)"
sudo ./tools/install-als.sh
```

The installer checks the exact laptop identity, installs the module through
DKMS, loads it, verifies the IIO device, and enables loading at boot. It does
not restart services or unload drivers. DKMS rebuilds the module for new
kernels when their headers are installed. Secure Boot requires the DKMS signing
key to be enrolled if the kernel rejects the module signature.

The app discovers the sensor automatically. A valid lux reading requires a
firmware sample; change the light reaching the sensor if it is still waiting.
Automatic brightness remains a separate opt-in COSMIC user service.

## Build/check only

Nothing in this directory installs or loads a module:

```sh
make check
# optional, after inspecting the result:
make clean
```

Use `KDIR=/path/to/kernel/build make check` for another kernel. The current
machine's `/lib/modules/$(uname -r)/build` headers are the intended check
target. Runtime verification still requires an explicit, separate load and
must confirm the IIO node and `cat in_illuminance_input`; this change does not
perform that operation.

## Design limits

- The first read returns `ENODATA` until firmware sends a valid WMI sample.
- The WMI notification is the sample clock. The module does not invent a
  kernel timer or query an undocumented method, and it does not implement an
  IIO threshold event.
- Consequently, normal IIO sysfs readers and `iio-sensor-proxy` polling are
  the compatibility path. A buffered IIO/event implementation would need a
  trigger, timestamp policy, and consumer support; adding it would not make
  current `iio-sensor-proxy` stop polling.
- The WMI callback is the kernel's deprecated ACPI-object callback API, used
  intentionally here because this firmware contract must validate the ACPI
  object type as well as its length and prefix. A future in-tree conversion
  should use `notify_new` only after preserving equivalent validation.

The sensor is independent of fan and power-profile control. The main app
uninstaller preserves this separately installed DKMS driver.
