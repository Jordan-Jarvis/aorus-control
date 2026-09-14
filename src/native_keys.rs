//! Privileged lifecycle for the native HID-BPF Fn-key translation.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use crate::fn_buttons::{FnAction, FnButtonMappings, PhysicalButtonId};

const INSTALL_PREFIX: &str = env!("AORUS_CONTROL_PREFIX");

fn helper_path() -> PathBuf {
    Path::new(INSTALL_PREFIX).join("libexec/aorus-brightness-hid-bpf")
}

fn object_path() -> PathBuf {
    Path::new(INSTALL_PREFIX).join("lib/aorus-control/0010-Gigabyte__AERO-16-YE5.bpf.o")
}
const ENABLED: &str = "/etc/aorus-control/brightness-hid-bpf.enabled";
const HID_DEVICES: &str = "/sys/bus/hid/devices";
const INPUT_DEVICES: &str = "/sys/class/input";
const BPFFS_ROOT: &str = "/sys/fs/bpf/hid";
const EV_KEY: u16 = 1;
const ACTION_MAP_VERSION: u8 = 2;
const ACTION_MAP_NAME: &str = "aorus_fn_act_v2";
const ACTION_MAP_ENTRIES: usize = 8;
const FIRST_IDENTITY_KEY: u16 = 183; // KEY_F13
const LAST_IDENTITY_KEY: u16 = 193; // KEY_F23
const NATIVE_BUTTONS: [PhysicalButtonId; 7] = [
    PhysicalButtonId::BrightnessDown,
    PhysicalButtonId::BrightnessUp,
    PhysicalButtonId::Fan,
    PhysicalButtonId::Sleep,
    PhysicalButtonId::Wifi,
    PhysicalButtonId::SquareX,
    PhysicalButtonId::Ai,
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeKeyStatus {
    pub supported: bool,
    pub enabled: bool,
    pub attached: bool,
    pub map_loaded: bool,
    pub map_generation: Option<u32>,
    pub reader_ready: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputEvent {
    time: libc::timeval,
    event_type: u16,
    code: u16,
    value: i32,
}

pub struct ActionInput {
    file: File,
    path: PathBuf,
}

impl ActionInput {
    /// HID-BPF reprobes can add a better translated input node without
    /// invalidating an existing file descriptor. Reopen when the preferred
    /// node changes instead of silently reading the old keyboard node.
    pub fn is_current(&self) -> bool {
        identity_input_path().is_ok_and(|path| path == self.path)
    }

    pub fn poll_action(&mut self) -> Result<Option<FnAction>, String> {
        loop {
            let mut event = InputEvent::default();
            let bytes = unsafe {
                std::slice::from_raw_parts_mut(
                    (&mut event as *mut InputEvent).cast::<u8>(),
                    std::mem::size_of::<InputEvent>(),
                )
            };
            match self.file.read(bytes) {
                Ok(0) => return Err("native Fn-key input device disappeared".to_owned()),
                Ok(size) if size != bytes.len() => {
                    return Err(format!(
                        "native Fn-key input returned a short event ({size}/{})",
                        bytes.len()
                    ));
                }
                Ok(_) if event.event_type == EV_KEY && event.value == 1 => {
                    if let Some(action) = daemon_action(event.code.into()) {
                        return Ok(Some(action));
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(format!("read native Fn-key input: {error}")),
            }
        }
    }
}

pub fn status() -> NativeKeyStatus {
    let devices = matching_devices().unwrap_or_default();
    let supported =
        exact_model() && helper_path().is_file() && object_path().is_file() && !devices.is_empty();
    let generation = map_generation();
    let attached = supported
        && devices
            .iter()
            .all(|device| helper("status", device).is_ok());
    NativeKeyStatus {
        supported,
        enabled: Path::new(ENABLED).is_file(),
        attached,
        map_generation: generation,
        map_loaded: generation.is_some(),
        reader_ready: attached && identity_input_path().is_ok(),
    }
}

fn map_path_for(hid: &Path) -> Option<PathBuf> {
    let sysname = hid.file_name()?.to_str()?.to_owned();
    Some(PathBuf::from(format!(
        "{BPFFS_ROOT}/{}/0010-Gigabyte__AERO-16-YE5_bpf/{ACTION_MAP_NAME}",
        sysname.replace([':', '.'], "_")
    )))
}

fn map_generation() -> Option<u32> {
    let hid = matching_devices().ok()?.into_iter().next()?;
    let path = map_path_for(&hid)?;
    if !path.is_file() {
        return None;
    }
    let fd = open_map(&path).ok()?;
    let value = map_lookup(fd, &7).ok();
    unsafe {
        libc::close(fd);
    }
    value
        .filter(|value| value.version == ACTION_MAP_VERSION && value.generation != 0)
        .map(|value| value.generation)
}

pub fn is_enabled() -> bool {
    Path::new(ENABLED).is_file()
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let devices = matching_devices()?;
    if !exact_model() || !helper_path().is_file() || !object_path().is_file() {
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

/// Restore a persistent attachment if the HID device or BPF link was lost.
/// This is safe to call periodically and after resume: an already healthy
/// attachment is left untouched.
pub fn repair_if_enabled() -> Result<bool, String> {
    if !Path::new(ENABLED).is_file() {
        return Ok(false);
    }
    let devices = matching_devices()?;
    if devices.is_empty() {
        return Err("the captured GIGABYTE HID interface is not present".to_owned());
    }

    let mut repaired = false;
    for device in devices {
        if helper("status", &device).is_err() {
            helper("recover", &device)?;
            helper("add", &device)?;
            helper("status", &device)?;
            repaired = true;
        }
    }
    Ok(repaired)
}

/// Apply mappings to the current exact HID input node and verify every write.
/// The device is selected by identity and private-key capability, never event number.
pub fn configure_mappings(mappings: &FnButtonMappings) -> Result<(), String> {
    update_native_map(mappings)
}

#[repr(C)]
#[derive(Clone, Copy, Default, Eq, PartialEq)]
struct BpfActionValue {
    version: u8,
    report_id: u8,
    payload: u16,
    generation: u32,
}

fn update_native_map(mappings: &FnButtonMappings) -> Result<(), String> {
    let hid = matching_devices()?
        .into_iter()
        .next()
        .ok_or_else(|| "the captured HID interface is not present".to_owned())?;
    let path = map_path_for(&hid).ok_or_else(|| "invalid native HID sysname".to_owned())?;
    let fd = open_map(&path)?;
    let result = (|| {
        let mut previous = [BpfActionValue::default(); ACTION_MAP_ENTRIES];
        for (index, value) in previous.iter_mut().enumerate() {
            *value = map_lookup(fd, &(index as u32))?;
        }
        let generation = previous[7].generation.wrapping_add(1).max(1);
        let mut next = [BpfActionValue::default(); ACTION_MAP_ENTRIES];
        for (index, button) in NATIVE_BUTTONS.into_iter().enumerate() {
            next[index] = action_value(mappings.get(button), generation)?;
        }
        next[7] = BpfActionValue {
            version: ACTION_MAP_VERSION,
            generation,
            ..Default::default()
        };
        for (index, value) in next.iter().enumerate() {
            if let Err(error) = map_update(fd, &(index as u32), value) {
                rollback_map(fd, &previous);
                return Err(format!(
                    "update native Fn map for {}: {error}",
                    if index < 7 {
                        NATIVE_BUTTONS[index].id()
                    } else {
                        "generation"
                    }
                ));
            }
        }
        for (index, expected) in next.iter().enumerate() {
            let observed = match map_lookup(fd, &(index as u32)) {
                Ok(value) => value,
                Err(error) => {
                    rollback_map(fd, &previous);
                    return Err(format!("read back native Fn map entry {index}: {error}"));
                }
            };
            if observed != *expected {
                rollback_map(fd, &previous);
                return Err(format!("native Fn map entry {index} readback mismatch"));
            }
        }
        Ok(())
    })();
    unsafe {
        libc::close(fd);
    }
    result
}

fn action_value(action: FnAction, generation: u32) -> Result<BpfActionValue, String> {
    let (report_id, payload) = match action {
        FnAction::BrightnessDown => (0x0a, 0x02),
        FnAction::BrightnessUp => (0x0a, 0x01),
        FnAction::Suspend => (0x0a, 0x04),
        FnAction::VolumeMute => (0x0a, 0x08),
        FnAction::VolumeUp => (0x0a, 0x10),
        FnAction::VolumeDown => (0x0a, 0x20),
        FnAction::MediaPlayPause => (0x0a, 0x40),
        FnAction::WifiToggle | FnAction::AirplaneToggle => (0x0b, 0x01),
        FnAction::Screenshot => (0x0c, 0x01),
        FnAction::Disabled => (0, 0),
        _ => (
            0x08,
            1u16 << daemon_action_code(action)
                .ok_or_else(|| format!("native HID action {} is unsupported", action.id()))?,
        ),
    };
    Ok(BpfActionValue {
        version: ACTION_MAP_VERSION,
        report_id,
        payload,
        generation,
    })
}

fn daemon_action_code(action: FnAction) -> Option<u32> {
    Some(match action {
        FnAction::PowerBattery => 0,
        FnAction::PowerBalanced => 1,
        FnAction::PowerPerformance => 2,
        FnAction::CyclePowerProfile => 3,
        FnAction::FanNormal => 4,
        FnAction::FanSilent => 5,
        FnAction::FanGaming => 6,
        FnAction::FanCustom => 7,
        FnAction::FanReapply => 8,
        FnAction::OpenApp => 9,
        FnAction::RunCommand => 10,
        _ => return None,
    })
}

#[repr(C)]
#[derive(Default)]
struct BpfObjGet {
    pathname: u64,
    bpf_fd: u32,
    file_flags: u32,
    path_fd: u32,
}

#[repr(C)]
struct BpfMapUpdate {
    map_fd: u32,
    key: u64,
    value: u64,
    flags: u64,
}

fn open_map(path: &Path) -> Result<i32, String> {
    let path_string = path.display().to_string();
    let cpath = std::ffi::CString::new(path_string.clone())
        .map_err(|_| "invalid BPF map path".to_owned())?;
    let obj = BpfObjGet {
        pathname: cpath.as_ptr() as u64,
        ..Default::default()
    };
    let fd = unsafe { bpf_syscall(7, &obj) };
    if fd < 0 {
        return Err(format!(
            "open pinned native Fn map {path_string}: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(fd)
}

fn map_update(fd: i32, key: &u32, value: &BpfActionValue) -> Result<(), String> {
    let attr = BpfMapUpdate {
        map_fd: fd as u32,
        key: key as *const _ as u64,
        value: value as *const _ as u64,
        flags: 0,
    };
    if unsafe { bpf_syscall(2, &attr) } != 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(())
}

fn rollback_map(fd: i32, previous: &[BpfActionValue; ACTION_MAP_ENTRIES]) {
    for (index, value) in previous.iter().enumerate() {
        let _ = map_update(fd, &(index as u32), value);
    }
}

fn map_lookup(fd: i32, key: &u32) -> Result<BpfActionValue, String> {
    let mut value = BpfActionValue::default();
    let attr = BpfMapUpdate {
        map_fd: fd as u32,
        key: key as *const _ as u64,
        value: &mut value as *mut _ as u64,
        flags: 0,
    };
    if unsafe { bpf_syscall(1, &attr) } != 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(value)
}

unsafe fn bpf_syscall<T>(command: u32, attr: &T) -> i32 {
    unsafe {
        libc::syscall(
            libc::SYS_bpf,
            command,
            attr as *const T,
            std::mem::size_of::<T>(),
        ) as i32
    }
}

pub fn mappings_active(mappings: &FnButtonMappings) -> bool {
    let Some(hid) = matching_devices().ok().and_then(|mut devices| {
        devices.sort();
        devices.into_iter().next()
    }) else {
        return false;
    };
    let Some(path) = map_path_for(&hid) else {
        return false;
    };
    let Ok(fd) = open_map(&path) else {
        return false;
    };
    let result = (|| {
        let marker = map_lookup(fd, &7).ok()?;
        if marker.version != ACTION_MAP_VERSION || marker.generation == 0 {
            return Some(false);
        }
        for (index, button) in NATIVE_BUTTONS.into_iter().enumerate() {
            let expected = action_value(mappings.get(button), marker.generation).ok()?;
            if map_lookup(fd, &(index as u32)).ok()? != expected {
                return Some(false);
            }
        }
        Some(true)
    })()
    .unwrap_or(false);
    unsafe {
        libc::close(fd);
    }
    result
}

pub fn action_input() -> Result<ActionInput, String> {
    let path = identity_input_path()?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path)
        .map_err(|error| format!("open native Fn-key input {}: {error}", path.display()))?;
    Ok(ActionInput { file, path })
}

fn daemon_action(keycode: u32) -> Option<FnAction> {
    match keycode.checked_sub(183) {
        Some(code @ 0..=10) => [
            FnAction::PowerBattery,
            FnAction::PowerBalanced,
            FnAction::PowerPerformance,
            FnAction::CyclePowerProfile,
            FnAction::FanNormal,
            FnAction::FanSilent,
            FnAction::FanGaming,
            FnAction::FanCustom,
            FnAction::FanReapply,
            FnAction::OpenApp,
            FnAction::RunCommand,
        ]
        .get(code as usize)
        .copied(),
        _ => None,
    }
}

fn identity_input_path() -> Result<PathBuf, String> {
    if !exact_model() {
        return Err("native Fn HID translation is unsupported on this model".to_owned());
    }
    let hid_devices = matching_devices()?;
    let mut selected: Option<(usize, PathBuf)> = None;
    for entry in fs::read_dir(INPUT_DEVICES)
        .map_err(|e| format!("read native input devices: {e}"))?
        .filter_map(Result::ok)
    {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("event") {
            continue;
        }
        let Ok(device) = fs::canonicalize(entry.path().join("device")) else {
            continue;
        };
        if !hid_devices
            .iter()
            .filter_map(|p| fs::canonicalize(p).ok())
            .any(|hid| device.starts_with(hid))
        {
            continue;
        }
        let capabilities = entry.path().join("device/capabilities/key");
        let Some(key_count) = identity_key_capability_count(&capabilities) else {
            continue;
        };
        let path = Path::new("/dev/input").join(&name);
        if path.exists()
            && selected
                .as_ref()
                .is_none_or(|(selected_count, _)| key_count < *selected_count)
        {
            selected = Some((key_count, path));
        }
    }
    selected
        .map(|(_, path)| path)
        .ok_or_else(|| "native Fn-key input device is unavailable".to_owned())
}

fn identity_key_capability_count(path: &Path) -> Option<usize> {
    let text = fs::read_to_string(path).ok()?;
    let words = text.split_whitespace().collect::<Vec<_>>();
    let has_all = (FIRST_IDENTITY_KEY..=LAST_IDENTITY_KEY).all(|code| {
        let word = usize::from(code / 64);
        let Some(index) = words.len().checked_sub(word + 1) else {
            return false;
        };
        let Ok(value) = u64::from_str_radix(words[index], 16) else {
            return false;
        };
        value & (1 << (code % 64)) != 0
    });
    has_all.then(|| {
        words
            .iter()
            .filter_map(|word| u64::from_str_radix(word, 16).ok())
            .map(u64::count_ones)
            .sum::<u32>() as usize
    })
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
    let output = Command::new(helper_path())
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
    use super::*;

    #[test]
    fn input_reader_drains_unrelated_events_and_ignores_release_and_repeat() {
        use std::io::Write;
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let mut descriptors = [0; 2];
        assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
        let reader = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        let mut writer = unsafe { File::from_raw_fd(descriptors[1]) };
        let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        let mut input = ActionInput {
            file: File::from(reader),
            path: PathBuf::new(),
        };
        for (code, value) in [(224, 1), (183, 0), (185, 2), (185, 1)] {
            let event = InputEvent {
                event_type: EV_KEY,
                code: code as u16,
                value,
                ..Default::default()
            };
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    (&event as *const InputEvent).cast::<u8>(),
                    std::mem::size_of::<InputEvent>(),
                )
            };
            writer.write_all(bytes).unwrap();
        }
        assert_eq!(
            input.poll_action().unwrap(),
            Some(FnAction::PowerPerformance)
        );
        assert_eq!(input.poll_action().unwrap(), None);
        drop(writer);
        assert!(input.poll_action().is_err());
    }

    #[test]
    fn daemon_action_codes_are_stable_and_distinct() {
        let actions = [
            FnAction::PowerBattery,
            FnAction::PowerBalanced,
            FnAction::PowerPerformance,
            FnAction::CyclePowerProfile,
            FnAction::FanNormal,
            FnAction::FanSilent,
            FnAction::FanGaming,
            FnAction::FanCustom,
            FnAction::FanReapply,
            FnAction::OpenApp,
            FnAction::RunCommand,
        ];
        for (code, action) in actions.into_iter().enumerate() {
            assert_eq!(daemon_action(183 + code as u32), Some(action));
        }
        assert_eq!(daemon_action(224), None);
    }

    #[test]
    fn only_accepts_exact_gigabyte_hid_names() {
        assert!(matching_device_name("0003:1044:7A3A.0004"));
        assert!(!matching_device_name("0003:1044:7A3A."));
        assert!(!matching_device_name("0003:1044:7A3B.0004"));
        assert!(!matching_device_name("0003:1044:7A3A.not-hex"));
    }
}
