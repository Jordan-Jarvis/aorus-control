//! Discovery and access for AORUS sysfs and hwmon nodes.

use crate::{
    curve::{CurveError, FAN_CURVE_POINTS, FanCurve, FanPoint},
    model::{Capabilities, DaemonMode, FanMode, Status},
};
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

const PLATFORM_NAME: &str = "aorus_laptop";
const HWMON_NAMES: &[&str] = &["aorus_laptop", "gigabyte_laptop", "gigabyte-laptop-wmi"];

pub trait HardwareIo: Send + Sync {
    fn read(&self, path: &Path) -> io::Result<String>;
    fn write(&self, path: &Path, value: &str) -> io::Result<()>;
}

struct FileIo;

impl HardwareIo for FileIo {
    fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write(&self, path: &Path, value: &str) -> io::Result<()> {
        fs::write(path, value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HardwarePaths {
    pub platform: Option<PathBuf>,
    pub hwmon: Option<PathBuf>,
}

impl HardwarePaths {
    pub fn discover(
        platform_root: impl AsRef<Path>,
        hwmon_root: impl AsRef<Path>,
    ) -> Result<Self, HardwareError> {
        let platform = find_named(platform_root.as_ref(), PLATFORM_NAME)?;
        if platform
            .as_ref()
            .is_none_or(|path| !path.join("fan_mode").is_file())
        {
            return Err(HardwareError::NotFound("aorus_laptop platform controls"));
        }
        let hwmon = find_hwmon(hwmon_root.as_ref())?;
        Ok(Self { platform, hwmon })
    }
}

pub struct Hardware {
    paths: HardwarePaths,
    io: Arc<dyn HardwareIo>,
    writes: Mutex<()>,
}

impl Hardware {
    pub fn discover() -> Result<Self, HardwareError> {
        Self::discover_at("/sys/devices/platform", "/sys/class/hwmon")
    }

    pub fn discover_at(
        platform_root: impl AsRef<Path>,
        hwmon_root: impl AsRef<Path>,
    ) -> Result<Self, HardwareError> {
        Self::from_paths(HardwarePaths::discover(platform_root, hwmon_root)?)
    }

    pub fn from_paths(paths: HardwarePaths) -> Result<Self, HardwareError> {
        Ok(Self {
            paths,
            io: Arc::new(FileIo),
            writes: Mutex::new(()),
        })
    }

    pub fn from_paths_with_io(paths: HardwarePaths, io: Arc<dyn HardwareIo>) -> Self {
        Self {
            paths,
            io,
            writes: Mutex::new(()),
        }
    }

    pub fn paths(&self) -> &HardwarePaths {
        &self.paths
    }

    pub fn capabilities(&self) -> Capabilities {
        self.detect_capabilities()
    }

    pub fn status(&self, daemon_mode: DaemonMode) -> Result<Status, HardwareError> {
        let _guard = self.guard()?;
        self.status_locked(daemon_mode)
    }

    pub fn read_curve(&self) -> Result<FanCurve, HardwareError> {
        let _guard = self.guard()?;
        self.read_curve_locked()
    }

    pub fn set_fan_mode(&self, mode: FanMode) -> Result<(), HardwareError> {
        self.with_write(|guard| guard.set_fan_mode(mode))
    }

    pub fn reselect_fan_mode(&self, mode: FanMode) -> Result<(), HardwareError> {
        self.with_write(|guard| guard.reselect_fan_mode(mode))
    }

    pub fn apply_curve(&self, curve: &FanCurve) -> Result<(), HardwareError> {
        self.with_write(|guard| guard.apply_curve(curve))
    }

    pub fn set_charge_mode(&self, mode: u8) -> Result<(), HardwareError> {
        self.with_write(|guard| guard.set_charge_mode(mode))
    }

    pub fn set_charge_limit(&self, limit: u8) -> Result<(), HardwareError> {
        self.with_write(|guard| guard.set_charge_limit(limit))
    }

    pub fn set_gpu_boost(&self, boost: u8) -> Result<(), HardwareError> {
        self.with_write(|guard| guard.set_gpu_boost(boost))
    }

    fn with_write<T>(
        &self,
        operation: impl FnOnce(&HardwareGuard<'_>) -> Result<T, HardwareError>,
    ) -> Result<T, HardwareError> {
        let guard = self.guard()?;
        operation(&guard)
    }

    pub fn guard(&self) -> Result<HardwareGuard<'_>, HardwareError> {
        self.writes
            .lock()
            .map(|lock| HardwareGuard {
                hardware: self,
                _lock: lock,
            })
            .map_err(|_| HardwareError::LockPoisoned)
    }

    fn status_locked(&self, daemon_mode: DaemonMode) -> Result<Status, HardwareError> {
        let platform = self.paths.platform.as_ref();
        let read_optional = |name: &str| {
            platform
                .map(|root| self.read_optional_number(&root.join(name)))
                .transpose()
                .map(|value| value.flatten())
        };
        let mut status = Status {
            daemon_mode,
            driver_available: platform.is_some(),
            capabilities: self.detect_capabilities(),
            ..Status::default()
        };
        status.fan_mode = read_optional("fan_mode")?
            .map(|value| {
                u8::try_from(value).map_err(|_| HardwareError::Value {
                    path: platform
                        .expect("fan_mode was read from a platform path")
                        .join("fan_mode"),
                    value: value.to_string(),
                })
            })
            .transpose()?
            .map(FanMode::try_from)
            .transpose()
            .map_err(HardwareError::Model)?;
        if status.fan_mode.is_none() {
            return Err(HardwareError::NotFound("aorus_laptop fan_mode control"));
        }
        status.charge_mode = read_optional("charge_mode")?.map(to_u8).transpose()?;
        status.charge_limit_percent = read_optional("charge_limit")?.map(to_u8).transpose()?;
        status.battery_cycles = read_optional("battery_cycle")?.map(to_u32).transpose()?;
        status.gpu_boost = read_optional("gpu_boost")?.map(to_u8).transpose()?;
        status.usb_charge_s3 = read_optional("usb_charge_s3_toggle")?.map(|value| value != 0);
        status.usb_charge_s4 = read_optional("usb_charge_s4_toggle")?.map(|value| value != 0);
        if let Some(hwmon) = self.paths.hwmon.as_ref() {
            for index in 0..3 {
                let value = self
                    .read_optional_number(&hwmon.join(format!("temp{}_input", index + 1)))?
                    .map(|value| {
                        i32::try_from(value).map_err(|_| HardwareError::Value {
                            path: hwmon.join(format!("temp{}_input", index + 1)),
                            value: value.to_string(),
                        })
                    })
                    .transpose()?;
                match index {
                    0 => status.temp1_millicelsius = value,
                    1 => status.temp2_millicelsius = value,
                    _ => status.temp3_millicelsius = value,
                }
            }
            for index in 0..4 {
                let value = self
                    .read_optional_number(&hwmon.join(format!("fan{}_input", index + 1)))?
                    .map(to_u32)
                    .transpose()?;
                let value = value.filter(|value| *value != 0);
                match index {
                    0 => status.fan1_rpm = value,
                    1 => status.fan2_rpm = value,
                    2 => status.fan3_rpm = value,
                    _ => status.fan4_rpm = value,
                }
            }
        }
        Ok(status)
    }

    fn detect_capabilities(&self) -> Capabilities {
        let Some(platform) = self.paths.platform.as_ref() else {
            return Capabilities::default();
        };
        let has = |name: &str| platform.join(name).is_file();
        Capabilities {
            fan_modes: if has("fan_mode") {
                FanMode::PROFILE_MODES.to_vec()
            } else {
                Vec::new()
            },
            curve_points: (has("fan_curve_index") && has("fan_curve_data"))
                .then_some(FAN_CURVE_POINTS as u8),
            charge_mode: has("charge_mode"),
            charge_limit: has("charge_limit"),
            // The driver documents the accepted wire range. A model-specific probe can narrow this later.
            // Presence proves telemetry support, not which model-specific values are safe.
            // Keep mutation disabled until this laptop's supported values are verified.
            gpu_boost_values: Vec::new(),
            usb_charge_s3: has("usb_charge_s3_toggle"),
            usb_charge_s4: has("usb_charge_s4_toggle"),
        }
    }

    fn read_curve_locked(&self) -> Result<FanCurve, HardwareError> {
        let platform = self.platform()?;
        let selector = platform.join("fan_curve_index");
        let data = platform.join("fan_curve_data");
        let old = self.read_number(&selector)?;
        let mut points = Vec::with_capacity(FAN_CURVE_POINTS);
        let result = (|| {
            for index in 0..FAN_CURVE_POINTS {
                self.write_text(&selector, &index.to_string())?;
                points.push(self.read_point(&data)?);
            }
            FanCurve::new(points).map_err(HardwareError::Curve)
        })();
        match self.write_text(&selector, &old.to_string()) {
            Ok(()) => result,
            Err(error) => Err(error),
        }
    }

    fn platform(&self) -> Result<&Path, HardwareError> {
        self.paths
            .platform
            .as_deref()
            .ok_or(HardwareError::NotFound("aorus_laptop platform controls"))
    }

    fn read_number(&self, path: &Path) -> Result<u64, HardwareError> {
        let value = self.io.read(path).map_err(|source| HardwareError::Io {
            operation: "read",
            path: path.to_owned(),
            source,
        })?;
        value.trim().parse().map_err(|_| HardwareError::Parse {
            path: path.to_owned(),
            value,
        })
    }

    fn read_point(&self, path: &Path) -> Result<FanPoint, HardwareError> {
        let value = self.io.read(path).map_err(|source| HardwareError::Io {
            operation: "read",
            path: path.to_owned(),
            source,
        })?;
        let mut values = value.split_whitespace();
        let temperature = values
            .next()
            .ok_or_else(|| HardwareError::Parse {
                path: path.to_owned(),
                value: value.clone(),
            })?
            .parse::<u8>()
            .map_err(|_| HardwareError::Parse {
                path: path.to_owned(),
                value: value.clone(),
            })?;
        let raw_speed = values
            .next()
            .ok_or_else(|| HardwareError::Parse {
                path: path.to_owned(),
                value: value.clone(),
            })?
            .parse::<u8>()
            .map_err(|_| HardwareError::Parse {
                path: path.to_owned(),
                value: value.clone(),
            })?;
        if values.next().is_some() {
            return Err(HardwareError::Parse {
                path: path.to_owned(),
                value,
            });
        }
        Ok(FanPoint::new(temperature, raw_speed))
    }

    fn read_optional_number(&self, path: &Path) -> Result<Option<u64>, HardwareError> {
        match self.io.read(path) {
            Ok(value) => value
                .trim()
                .parse()
                .map(Some)
                .map_err(|_| HardwareError::Parse {
                    path: path.to_owned(),
                    value,
                }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(HardwareError::Io {
                operation: "read",
                path: path.to_owned(),
                source,
            }),
        }
    }

    fn write_text(&self, path: &Path, value: &str) -> Result<(), HardwareError> {
        self.io
            .write(path, value)
            .map_err(|source| HardwareError::Io {
                operation: "write",
                path: path.to_owned(),
                source,
            })
    }
}

pub struct HardwareGuard<'a> {
    hardware: &'a Hardware,
    _lock: MutexGuard<'a, ()>,
}

impl HardwareGuard<'_> {
    pub fn set_fan_mode(&self, mode: FanMode) -> Result<(), HardwareError> {
        if !mode.is_profile() {
            return Err(HardwareError::UnsafeFanMode(mode));
        }
        self.write_fan_mode(mode)
    }

    pub fn reselect_fan_mode(&self, mode: FanMode) -> Result<(), HardwareError> {
        if !mode.is_profile() {
            return Err(HardwareError::UnsafeFanMode(mode));
        }
        let current = self.read_fan_mode()?;
        if current == mode {
            let detour = if mode == FanMode::Gaming {
                FanMode::Normal
            } else {
                FanMode::Gaming
            };
            self.write_fan_mode(detour)?;
        }
        self.write_fan_mode(mode)
    }

    pub fn apply_curve(&self, proposed: &FanCurve) -> Result<(), HardwareError> {
        proposed.validate().map_err(HardwareError::Curve)?;
        let previous_curve = self.hardware.read_curve_locked()?;
        let previous_mode = self.read_fan_mode()?;
        if previous_mode != FanMode::Gaming {
            self.write_fan_mode(FanMode::Gaming)?;
        }

        let write_result = self.write_curve(proposed);
        let readback = write_result
            .and_then(|()| self.hardware.read_curve_locked())
            .and_then(|actual| {
                if actual == *proposed {
                    Ok(())
                } else {
                    Err(HardwareError::Verification {
                        expected: "proposed curve",
                        actual: "readback differs",
                    })
                }
            });
        let commit = readback.and_then(|()| self.write_fan_mode(FanMode::Custom));
        if commit.is_ok() {
            return Ok(());
        }

        let failure = commit.unwrap_err();
        let rollback = self.write_curve(&previous_curve).and_then(|()| {
            let actual = self.hardware.read_curve_locked()?;
            if actual != previous_curve {
                return Err(HardwareError::Verification {
                    expected: "previous curve",
                    actual: "rollback readback differs",
                });
            }
            // Restore exactly what firmware reported, including Auto/Fixed if
            // an external tool had selected it. This rollback never writes a
            // hard-coded fan speed.
            self.write_fan_mode(previous_mode)?;
            Ok(())
        });
        match rollback {
            Ok(()) => Err(HardwareError::Transaction {
                failure: failure.to_string(),
                rollback_verified: true,
            }),
            Err(rollback) => {
                // Firmware Gaming is the safe fallback. Report loudly if even that
                // verified transition fails; never claim a partial curve is safe.
                let fallback = self.write_fan_mode(FanMode::Gaming);
                let fallback_detail = match fallback {
                    Ok(()) => "Gaming fallback verified".to_owned(),
                    Err(error) => format!(
                        "CRITICAL: Gaming fallback could not be verified ({error}); manual recovery required"
                    ),
                };
                Err(HardwareError::Transaction {
                    failure: format!("{failure}; rollback failed: {rollback}; {fallback_detail}"),
                    rollback_verified: false,
                })
            }
        }
    }

    pub fn set_charge_mode(&self, mode: u8) -> Result<(), HardwareError> {
        if mode > 1 {
            return Err(HardwareError::InvalidChargeMode(mode));
        }
        self.checked_write("charge_mode", mode, mode as u64)
    }

    pub fn set_charge_limit(&self, limit: u8) -> Result<(), HardwareError> {
        if !(60..=100).contains(&limit) {
            return Err(HardwareError::InvalidChargeLimit(limit));
        }
        self.checked_write("charge_limit", limit, limit as u64)
    }

    pub fn set_gpu_boost(&self, boost: u8) -> Result<(), HardwareError> {
        if !self
            .hardware
            .detect_capabilities()
            .gpu_boost_values
            .contains(&boost)
        {
            return Err(HardwareError::InvalidGpuBoost(boost));
        }
        self.checked_write("gpu_boost", boost, boost as u64)
    }

    fn write_curve(&self, curve: &FanCurve) -> Result<(), HardwareError> {
        let platform = self.hardware.platform()?;
        let selector = platform.join("fan_curve_index");
        let data = platform.join("fan_curve_data");
        let old = self.hardware.read_number(&selector)?;
        let result = (|| {
            for (index, point) in curve.points().iter().enumerate() {
                self.hardware.write_text(&selector, &index.to_string())?;
                self.hardware
                    .write_text(&data, &point.packed().to_string())?;
            }
            Ok(())
        })();
        match self.hardware.write_text(&selector, &old.to_string()) {
            Ok(()) => result,
            Err(error) => Err(error),
        }
    }

    fn write_fan_mode(&self, mode: FanMode) -> Result<(), HardwareError> {
        let path = self.hardware.platform()?.join("fan_mode");
        self.hardware.write_text(&path, &mode.as_u8().to_string())?;
        let actual = self.read_fan_mode()?;
        if actual != mode {
            return Err(HardwareError::Verification {
                expected: "fan mode",
                actual: "readback differs",
            });
        }
        Ok(())
    }

    fn read_fan_mode(&self) -> Result<FanMode, HardwareError> {
        let path = self.hardware.platform()?.join("fan_mode");
        let value = self.hardware.read_number(&path)?;
        let value = u8::try_from(value).map_err(|_| HardwareError::Value {
            path: path.clone(),
            value: value.to_string(),
        })?;
        FanMode::try_from(value).map_err(HardwareError::Model)
    }

    fn checked_write(&self, name: &str, value: u8, expected: u64) -> Result<(), HardwareError> {
        let path = self.hardware.platform()?.join(name);
        self.hardware.write_text(&path, &value.to_string())?;
        let actual = self.hardware.read_number(&path)?;
        if actual != expected {
            return Err(HardwareError::Verification {
                expected: "requested value",
                actual: "readback differs",
            });
        }
        Ok(())
    }
}

fn find_named(root: &Path, name: &str) -> Result<Option<PathBuf>, HardwareError> {
    if !root.exists() {
        return Ok(None);
    }
    if root.file_name().and_then(|value| value.to_str()) == Some(name)
        || fs::read_to_string(root.join("name"))
            .map(|value| value.trim().eq_ignore_ascii_case(name))
            .unwrap_or(false)
    {
        return Ok(Some(root.to_owned()));
    }
    for entry in fs::read_dir(root).map_err(|source| HardwareError::Io {
        operation: "scan",
        path: root.to_owned(),
        source,
    })? {
        let entry = entry.map_err(|source| HardwareError::Io {
            operation: "scan",
            path: root.to_owned(),
            source,
        })?;
        let path = entry.path();
        if path.file_name().and_then(|value| value.to_str()) == Some(name)
            || fs::read_to_string(path.join("name"))
                .map(|value| value.trim().eq_ignore_ascii_case(name))
                .unwrap_or(false)
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn find_hwmon(root: &Path) -> Result<Option<PathBuf>, HardwareError> {
    if !root.exists() {
        return Ok(None);
    }
    for entry in fs::read_dir(root).map_err(|source| HardwareError::Io {
        operation: "scan",
        path: root.to_owned(),
        source,
    })? {
        let entry = entry.map_err(|source| HardwareError::Io {
            operation: "scan",
            path: root.to_owned(),
            source,
        })?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(name) = fs::read_to_string(path.join("name")) else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if HWMON_NAMES.iter().any(|expected| name == *expected)
            || name.contains("aorus")
            || name.contains("gigabyte")
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn to_u8(value: u64) -> Result<u8, HardwareError> {
    u8::try_from(value).map_err(|_| HardwareError::Value {
        path: PathBuf::new(),
        value: value.to_string(),
    })
}

fn to_u32(value: u64) -> Result<u32, HardwareError> {
    u32::try_from(value).map_err(|_| HardwareError::Value {
        path: PathBuf::new(),
        value: value.to_string(),
    })
}

#[derive(Debug)]
pub enum HardwareError {
    NotFound(&'static str),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        value: String,
    },
    Value {
        path: PathBuf,
        value: String,
    },
    LockPoisoned,
    Model(crate::model::ModelError),
    Curve(CurveError),
    UnsafeFanMode(FanMode),
    InvalidChargeMode(u8),
    InvalidChargeLimit(u8),
    InvalidGpuBoost(u8),
    Verification {
        expected: &'static str,
        actual: &'static str,
    },
    Transaction {
        failure: String,
        rollback_verified: bool,
    },
}

impl fmt::Display for HardwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(what) => write!(f, "{what} not found"),
            Self::Io {
                operation,
                path,
                source,
            } => write!(f, "{operation} {}: {source}", path.display()),
            Self::Parse { path, .. } => write!(f, "invalid numeric value in {}", path.display()),
            Self::Value { path, value } => {
                write!(f, "value '{value}' is out of range for {}", path.display())
            }
            Self::LockPoisoned => f.write_str("hardware lock is poisoned"),
            Self::Model(error) => error.fmt(f),
            Self::Curve(error) => error.fmt(f),
            Self::UnsafeFanMode(mode) => write!(f, "fan mode {mode} is not a profile mode"),
            Self::InvalidChargeMode(value) => {
                write!(f, "invalid charge mode {value}; expected 0 or 1")
            }
            Self::InvalidChargeLimit(value) => {
                write!(f, "invalid charge limit {value}; expected 60..=100")
            }
            Self::InvalidGpuBoost(value) => {
                write!(
                    f,
                    "GPU boost value {value} is unsupported or unverified on this model"
                )
            }
            Self::Verification { expected, actual } => {
                write!(f, "verification failed: expected {expected}, {actual}")
            }
            Self::Transaction {
                failure,
                rollback_verified,
            } => write!(
                f,
                "curve transaction failed ({failure}); rollback verified={rollback_verified}"
            ),
        }
    }
}

impl std::error::Error for HardwareError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "aorus-control-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(path: &Path, value: impl AsRef<[u8]>) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, value).unwrap();
    }

    struct FakeIo {
        values: Mutex<HashMap<PathBuf, String>>,
        points: Mutex<[FanPoint; FAN_CURVE_POINTS]>,
        selected: Mutex<usize>,
        fail_once_at: Mutex<Option<usize>>,
        fail_always_at: Mutex<Option<usize>>,
    }

    struct DenyIo;

    impl HardwareIo for DenyIo {
        fn read(&self, path: &Path) -> io::Result<String> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                path.display().to_string(),
            ))
        }

        fn write(&self, path: &Path, _value: &str) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                path.display().to_string(),
            ))
        }
    }

    impl FakeIo {
        fn new(points: [FanPoint; FAN_CURVE_POINTS]) -> Self {
            Self {
                values: Mutex::new(HashMap::new()),
                points: Mutex::new(points),
                selected: Mutex::new(0),
                fail_once_at: Mutex::new(None),
                fail_always_at: Mutex::new(None),
            }
        }

        fn fail_once_at(&self, index: usize) {
            *self.fail_once_at.lock().unwrap() = Some(index);
        }

        fn fail_always_at(&self, index: usize) {
            *self.fail_always_at.lock().unwrap() = Some(index);
        }
    }

    impl HardwareIo for FakeIo {
        fn read(&self, path: &Path) -> io::Result<String> {
            if path.file_name().and_then(|name| name.to_str()) == Some("fan_curve_data") {
                let index = *self.selected.lock().unwrap();
                let point = self.points.lock().unwrap()[index];
                return Ok(format!("{} {}\n", point.temperature, point.raw_speed));
            }
            self.values
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.display().to_string()))
        }

        fn write(&self, path: &Path, value: &str) -> io::Result<()> {
            let name = path.file_name().and_then(|name| name.to_str());
            if name == Some("fan_curve_index") {
                let index = value.trim().parse::<usize>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid curve index")
                })?;
                *self.selected.lock().unwrap() = index;
            } else if name == Some("fan_curve_data") {
                let index = *self.selected.lock().unwrap();
                let fail = *self.fail_always_at.lock().unwrap() == Some(index);
                let fail = if fail {
                    true
                } else if *self.fail_once_at.lock().unwrap() == Some(index) {
                    *self.fail_once_at.lock().unwrap() = None;
                    true
                } else {
                    false
                };
                if fail {
                    return Err(io::Error::other("injected curve write failure"));
                }
                let packed = value.trim().parse::<u16>().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid packed curve value")
                })?;
                self.points.lock().unwrap()[index] = FanPoint::from_packed(packed);
            }
            self.values
                .lock()
                .unwrap()
                .insert(path.to_owned(), value.to_owned());
            Ok(())
        }
    }

    fn fake_fixture() -> (PathBuf, Hardware, Arc<FakeIo>) {
        let root = temp_dir();
        let platform = root.join("devices/platform/aorus_laptop");
        let hwmon = root.join("class/hwmon/hwmon17");
        for name in [
            "fan_mode",
            "charge_mode",
            "charge_limit",
            "gpu_boost",
            "fan_curve_index",
            "battery_cycle",
            "usb_charge_s3_toggle",
            "usb_charge_s4_toggle",
            "fan_curve_data",
        ] {
            write(&platform.join(name), "0");
        }
        write(&hwmon.join("name"), "aorus_laptop\n");
        for index in 0..3 {
            write(&hwmon.join(format!("temp{}_input", index + 1)), "40000");
        }
        for index in 0..4 {
            write(&hwmon.join(format!("fan{}_input", index + 1)), "4000");
        }
        let points = std::array::from_fn(|index| FanPoint::new(index as u8 * 5, index as u8 * 10));
        let fake = Arc::new(FakeIo::new(points));
        fake.values
            .lock()
            .unwrap()
            .insert(platform.join("fan_mode"), "0".to_owned());
        fake.values
            .lock()
            .unwrap()
            .insert(platform.join("charge_mode"), "0".to_owned());
        fake.values
            .lock()
            .unwrap()
            .insert(platform.join("charge_limit"), "97".to_owned());
        fake.values
            .lock()
            .unwrap()
            .insert(platform.join("gpu_boost"), "0".to_owned());
        fake.values
            .lock()
            .unwrap()
            .insert(platform.join("fan_curve_index"), "0".to_owned());
        let paths = HardwarePaths {
            platform: Some(platform),
            hwmon: Some(hwmon),
        };
        let hardware = Hardware::from_paths_with_io(paths, fake.clone());
        (root, hardware, fake)
    }

    fn fixture() -> (PathBuf, Hardware) {
        let root = temp_dir();
        let platform_root = root.join("devices/platform");
        let platform = platform_root.join("aorus_laptop");
        let hwmon_root = root.join("class/hwmon");
        let hwmon = hwmon_root.join("hwmon9");
        for (name, value) in [
            ("fan_mode", "2"),
            ("charge_mode", "0"),
            ("charge_limit", "97"),
            ("gpu_boost", "0"),
            ("fan_curve_index", "0"),
            ("battery_cycle", "145"),
            ("usb_charge_s3_toggle", "0"),
            ("usb_charge_s4_toggle", "1"),
        ] {
            write(&platform.join(name), value);
        }
        write(&hwmon.join("name"), "gigabyte_laptop\n");
        for index in 0..3 {
            write(
                &hwmon.join(format!("temp{}_input", index + 1)),
                (40000 + index * 1000).to_string(),
            );
        }
        for index in 0..4 {
            write(
                &hwmon.join(format!("fan{}_input", index + 1)),
                if index < 2 {
                    (4000 + index * 1000).to_string()
                } else {
                    "0".to_owned()
                },
            );
        }
        for index in 0..FAN_CURVE_POINTS {
            write(&platform.join(format!("curve-{index}")), "");
        }
        write(&platform.join("fan_curve_data"), "0");
        let hardware = Hardware::discover_at(platform_root, hwmon_root).unwrap();
        (root, hardware)
    }

    #[test]
    fn discovers_by_platform_and_hwmon_name_not_number() {
        let (root, hardware) = fixture();
        assert!(
            hardware
                .paths()
                .platform
                .as_ref()
                .unwrap()
                .ends_with("aorus_laptop")
        );
        assert!(hardware.paths().hwmon.as_ref().unwrap().ends_with("hwmon9"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn platform_discovery_is_shallow_and_requires_controls() {
        let root = temp_dir();
        let platform_root = root.join("devices/platform");
        let hwmon_root = root.join("class/hwmon");
        write(
            &platform_root.join("unrelated/nested/aorus_laptop/fan_mode"),
            "0",
        );
        assert!(matches!(
            Hardware::discover_at(&platform_root, &hwmon_root),
            Err(HardwareError::NotFound("aorus_laptop platform controls"))
        ));

        write(&platform_root.join("aorus_laptop/fan_mode"), "0");
        assert!(Hardware::discover_at(&platform_root, &hwmon_root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn status_omits_missing_channels_and_reads_units() {
        let (root, hardware) = fixture();
        fs::remove_file(hardware.paths().hwmon.as_ref().unwrap().join("fan3_input")).unwrap();
        let status = hardware.status(DaemonMode::Shadow).unwrap();
        assert_eq!(status.temp1_millicelsius, Some(40000));
        assert_eq!(status.fan1_rpm, Some(4000));
        assert_eq!(status.fan3_rpm, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_writes_reject_fixed_and_reselect_same_mode() {
        let (root, hardware) = fixture();
        assert!(matches!(
            hardware.set_fan_mode(FanMode::Fixed),
            Err(HardwareError::UnsafeFanMode(FanMode::Fixed))
        ));
        hardware.reselect_fan_mode(FanMode::Gaming).unwrap();
        assert_eq!(
            fs::read_to_string(hardware.paths().platform.as_ref().unwrap().join("fan_mode"))
                .unwrap(),
            "2"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn charge_and_gpu_values_are_validated() {
        let (root, hardware) = fixture();
        assert!(matches!(
            hardware.set_charge_limit(59),
            Err(HardwareError::InvalidChargeLimit(59))
        ));
        assert!(matches!(
            hardware.set_charge_mode(2),
            Err(HardwareError::InvalidChargeMode(2))
        ));
        assert!(matches!(
            hardware.set_gpu_boost(4),
            Err(HardwareError::InvalidGpuBoost(4))
        ));
        assert!(matches!(
            hardware.set_gpu_boost(0),
            Err(HardwareError::InvalidGpuBoost(0))
        ));
        hardware.set_charge_limit(80).unwrap();
        assert_eq!(
            fs::read_to_string(
                hardware
                    .paths()
                    .platform
                    .as_ref()
                    .unwrap()
                    .join("charge_limit")
            )
            .unwrap(),
            "80"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_curve_is_rejected_when_explicitly_read() {
        let (root, hardware) = fixture();
        write(
            &hardware
                .paths()
                .platform
                .as_ref()
                .unwrap()
                .join("fan_curve_data"),
            "not-a-number",
        );
        assert!(hardware.read_curve().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disappearing_and_denied_curve_nodes_are_reported() {
        let (root, hardware) = fixture();
        fs::remove_file(
            hardware
                .paths()
                .platform
                .as_ref()
                .unwrap()
                .join("fan_curve_index"),
        )
        .unwrap();
        assert!(matches!(
            hardware.read_curve(),
            Err(HardwareError::Io { .. })
        ));

        let denied = Hardware::from_paths_with_io(hardware.paths().clone(), Arc::new(DenyIo));
        assert!(matches!(
            denied.status(DaemonMode::Shadow),
            Err(HardwareError::Io { .. })
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn curve_transaction_writes_exactly_and_activates_custom() {
        let (root, hardware, fake) = fake_fixture();
        *fake.selected.lock().unwrap() = 4;
        fake.values.lock().unwrap().insert(
            hardware
                .paths()
                .platform
                .as_ref()
                .unwrap()
                .join("fan_curve_index"),
            "4".to_owned(),
        );
        let proposed = FanCurve::from_valid_array(std::array::from_fn(|index| {
            FanPoint::new(index as u8 * 6, index as u8 * 12)
        }));
        hardware.apply_curve(&proposed).unwrap();
        assert_eq!(hardware.read_curve().unwrap(), proposed);
        assert_eq!(*fake.selected.lock().unwrap(), 4);
        assert_eq!(
            hardware.status(DaemonMode::WriteEnabled).unwrap().fan_mode,
            Some(FanMode::Custom)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_curve_write_rolls_back_and_never_activates_partial_curve() {
        for failed_index in 0..FAN_CURVE_POINTS {
            let (root, hardware, fake) = fake_fixture();
            let original = hardware.read_curve().unwrap();
            let proposed = FanCurve::from_valid_array(std::array::from_fn(|index| {
                FanPoint::new(index as u8 * 6, index as u8 * 12)
            }));
            fake.fail_once_at(failed_index);
            assert!(matches!(
                hardware.apply_curve(&proposed),
                Err(HardwareError::Transaction {
                    rollback_verified: true,
                    ..
                })
            ));
            assert_eq!(hardware.read_curve().unwrap(), original);
            assert_eq!(
                hardware.status(DaemonMode::WriteEnabled).unwrap().fan_mode,
                Some(FanMode::Normal)
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn failed_curve_write_restores_an_externally_selected_auto_mode() {
        let (root, hardware, fake) = fake_fixture();
        let platform = hardware.paths().platform.as_ref().unwrap();
        fake.values
            .lock()
            .unwrap()
            .insert(platform.join("fan_mode"), "4".to_owned());
        let proposed = FanCurve::from_valid_array(std::array::from_fn(|index| {
            FanPoint::new(index as u8 * 6, index as u8 * 12)
        }));
        fake.fail_once_at(4);
        assert!(matches!(
            hardware.apply_curve(&proposed),
            Err(HardwareError::Transaction {
                rollback_verified: true,
                ..
            })
        ));
        assert_eq!(
            hardware.status(DaemonMode::WriteEnabled).unwrap().fan_mode,
            Some(FanMode::Auto)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_rollback_leaves_firmware_gaming() {
        let (root, hardware, fake) = fake_fixture();
        let proposed = FanCurve::from_valid_array(std::array::from_fn(|index| {
            FanPoint::new(index as u8 * 6, index as u8 * 12)
        }));
        fake.fail_always_at(5);
        assert!(matches!(
            hardware.apply_curve(&proposed),
            Err(HardwareError::Transaction {
                rollback_verified: false,
                ..
            })
        ));
        assert_eq!(
            hardware.status(DaemonMode::WriteEnabled).unwrap().fan_mode,
            Some(FanMode::Gaming)
        );
        fs::remove_dir_all(root).unwrap();
    }
}
