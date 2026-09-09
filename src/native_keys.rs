//! Privileged lifecycle for the native HID-BPF Fn-key translation.

use std::{
    fs::{self, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const HELPER: &str = "/usr/local/libexec/aorus-brightness-hid-bpf";
const OBJECT: &str = "/usr/local/lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o";
const ENABLED: &str = "/etc/aorus-control/brightness-hid-bpf.enabled";
const HID_DEVICES: &str = "/sys/bus/hid/devices";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeKeyStatus {
    pub supported: bool,
    pub enabled: bool,
    pub active: bool,
}

pub fn status() -> NativeKeyStatus {
    let devices = matching_devices().unwrap_or_default();
    let supported = exact_model()
        && Path::new(HELPER).is_file()
        && Path::new(OBJECT).is_file()
        && !devices.is_empty();
    NativeKeyStatus {
        supported,
        enabled: Path::new(ENABLED).is_file(),
        active: supported
            && devices
                .iter()
                .all(|device| helper("status", device).is_ok()),
    }
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let devices = matching_devices()?;
    if !exact_model() || !Path::new(HELPER).is_file() || !Path::new(OBJECT).is_file() {
        return Err("native Fn-key support is not installed for this laptop".to_owned());
    }
    if devices.is_empty() {
        return Err("the captured GIGABYTE HID interface is not present".to_owned());
    }

    if !enabled {
        match fs::remove_file(ENABLED) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("remove {ENABLED}: {error}")),
        }
        for device in devices {
            helper("recover", &device)?;
        }
        return Ok(());
    }

    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(ENABLED)
        .map_err(|error| format!("create {ENABLED}: {error}"))?;

    for device in &devices {
        if let Err(error) = helper("add", device).and_then(|()| helper("status", device)) {
            let _ = fs::remove_file(ENABLED);
            for device in &devices {
                let _ = helper("recover", device);
            }
            return Err(error);
        }
    }
    Ok(())
}

fn exact_model() -> bool {
    read_trimmed("/sys/class/dmi/id/sys_vendor").as_deref() == Some("GIGABYTE")
        && read_trimmed("/sys/class/dmi/id/product_name").as_deref() == Some("AERO 16 YE5")
        && read_trimmed("/sys/class/dmi/id/product_version").as_deref() == Some("P86VE")
}

fn matching_devices() -> Result<Vec<PathBuf>, String> {
    let entries =
        fs::read_dir(HID_DEVICES).map_err(|error| format!("read native HID devices: {error}"))?;
    let mut devices = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(matching_device_name)
                && read_trimmed(path.join("../bInterfaceNumber")).as_deref() == Some("02")
        })
        .collect::<Vec<_>>();
    devices.sort();
    Ok(devices)
}

fn matching_device_name(name: &str) -> bool {
    name.strip_prefix("0003:1044:7A3A.").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.chars().all(|value| value.is_ascii_hexdigit())
    })
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn helper(mode: &str, device: &Path) -> Result<(), String> {
    let output = Command::new(HELPER)
        .arg(mode)
        .arg(device)
        .output()
        .map_err(|error| format!("run native Fn-key helper: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if detail.is_empty() {
        format!("native Fn-key helper {mode} failed with {}", output.status)
    } else {
        format!("native Fn-key helper {mode} failed: {detail}")
    })
}

#[cfg(test)]
mod tests {
    use super::matching_device_name;

    #[test]
    fn only_accepts_exact_gigabyte_hid_names() {
        assert!(matching_device_name("0003:1044:7A3A.0004"));
        assert!(!matching_device_name("0003:1044:7A3A."));
        assert!(!matching_device_name("0003:1044:7A3B.0004"));
        assert!(!matching_device_name("0003:1044:7A3A.not-hex"));
    }
}
