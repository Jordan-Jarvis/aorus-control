//! System-bus API and the daemon's private orchestration layer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::{Connection, Proxy, connection::Builder};
use zbus::interface;
use zbus::message::Header;
use zbus::zvariant::{Array, OwnedObjectPath, OwnedValue, Value};

use crate::config::Config;
use crate::curve::{FanCurve, FanPoint};
use crate::hardware::{Hardware, HardwareError};
use crate::model::{DaemonMode, FanMode, PowerProfile, Status};
use crate::native_keys;
use crate::profile;

const POLKIT_ACTION: &str = "io.github.aoruslinux.control.modify";
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(60);
const RESUME_REAPPLY_DELAY: Duration = Duration::from_secs(4);
const POWER_REAPPLY_DELAY: Duration = Duration::from_secs(2);
const METHOD_TIMEOUT: Duration = Duration::from_secs(10);
const CURVE_CACHE_TTL: Duration = Duration::from_secs(2);

fn system_connection() -> Result<Connection, zbus::Error> {
    Builder::system()?.method_timeout(METHOD_TIMEOUT).build()
}

struct State {
    mode: DaemonMode,
    hardware: Option<Arc<Hardware>>,
    config: Config,
    config_path: PathBuf,
    last_error: Option<String>,
    last_profile: Option<(PowerProfile, Instant)>,
    curve_cache: Option<(FanCurve, Instant)>,
}

type StateSnapshot = (DaemonMode, Option<Arc<Hardware>>, Config, Option<String>);

/// The object exported by `aorusd`. It is also cheaply cloned into watcher threads.
#[derive(Clone)]
pub struct AorusControl {
    state: Arc<Mutex<State>>,
    mutations: Arc<Mutex<()>>,
}

impl AorusControl {
    pub fn new(write_enabled: bool) -> Result<Self, String> {
        let config_path = std::env::var_os("AORUS_CONTROL_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/etc/aorus-control/config.toml"));
        let (config, config_error) = if config_path.exists() {
            match Config::load(&config_path) {
                Ok(config) => (config, None),
                Err(error) if write_enabled => {
                    return Err(format!(
                        "refusing write-enabled startup with invalid configuration: {error}"
                    ));
                }
                Err(error) => (Config::default(), Some(format!("configuration: {error}"))),
            }
        } else {
            (Config::default(), None)
        };

        let (hardware, hardware_error) = match discover_hardware() {
            Ok(hardware) => (Some(Arc::new(hardware)), None),
            Err(error) => (None, Some(format!("hardware discovery: {error}"))),
        };
        let last_error = config_error.or(hardware_error);

        Ok(Self {
            state: Arc::new(Mutex::new(State {
                mode: if write_enabled {
                    DaemonMode::WriteEnabled
                } else {
                    DaemonMode::Shadow
                },
                hardware,
                config,
                config_path,
                last_error,
                last_profile: None,
                curve_cache: None,
            })),
            mutations: Arc::new(Mutex::new(())),
        })
    }

    fn snapshot(&self) -> Result<StateSnapshot, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?;
        Ok((
            state.mode,
            state.hardware.clone(),
            state.config.clone(),
            state.last_error.clone(),
        ))
    }

    fn remember_error(&self, error: impl Into<String>) -> String {
        let error = error.into();
        eprintln!("aorusd error: {error}");
        if let Ok(mut state) = self.state.lock() {
            state.last_error = Some(error.clone());
        }
        error
    }

    fn clear_error(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.last_error = None;
        }
    }

    fn clear_hardware_error(&self) {
        if let Ok(mut state) = self.state.lock()
            && state
                .last_error
                .as_deref()
                .is_some_and(|error| error.starts_with("hardware "))
        {
            state.last_error = None;
        }
    }

    fn forget_hardware(&self, error: impl Into<String>) -> String {
        let error = error.into();
        if let Ok(mut state) = self.state.lock() {
            state.hardware = None;
            state.curve_cache = None;
        }
        self.remember_error(error)
    }

    fn require_write_mode(&self) -> Result<(), String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?;
        if state.mode == DaemonMode::Shadow {
            Err("daemon is in shadow mode; Python remains authoritative".to_owned())
        } else if python_service_active()? {
            Err(
                "refusing Rust hardware writes while aorus-power-profile-sync.service is active"
                    .to_owned(),
            )
        } else {
            Ok(())
        }
    }

    fn hardware(&self) -> Result<Arc<Hardware>, String> {
        if let Some(hardware) = self
            .state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?
            .hardware
            .clone()
        {
            return Ok(hardware);
        }
        let hardware = Arc::new(
            discover_hardware()
                .map_err(|error| self.remember_error(format!("hardware discovery: {error}")))?,
        );
        self.state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?
            .hardware = Some(hardware.clone());
        self.clear_hardware_error();
        Ok(hardware)
    }

    fn config_path(&self) -> Result<PathBuf, String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?
            .config_path
            .clone())
    }

    fn mapped_mode(&self, profile: PowerProfile) -> Result<FanMode, String> {
        self.state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?
            .config
            .profile_mappings
            .get(&profile)
            .copied()
            .ok_or_else(|| format!("no fan mapping for {profile}"))
    }

    fn cached_curve(&self) -> Option<FanCurve> {
        self.state.lock().ok().and_then(|state| {
            state
                .curve_cache
                .as_ref()
                .filter(|(_, at)| at.elapsed() < CURVE_CACHE_TTL)
                .map(|(curve, _)| curve.clone())
        })
    }

    fn remember_curve(&self, curve: FanCurve) {
        if let Ok(mut state) = self.state.lock() {
            state.curve_cache = Some((curve, Instant::now()));
        }
    }

    fn disable_custom_curve(&self) -> Result<(), String> {
        let mut config = self.snapshot()?.2;
        config.invalidate_custom_curve();
        config
            .save_atomic(self.config_path()?)
            .map_err(|error| error.to_string())?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?;
        state.config = config;
        state.curve_cache = None;
        Ok(())
    }

    fn apply_stored_curve(&self, curve: &FanCurve) -> Result<(), String> {
        match self.hardware()?.apply_curve(curve) {
            Ok(()) => {
                self.remember_curve(curve.clone());
                Ok(())
            }
            Err(
                error @ HardwareError::Transaction {
                    rollback_verified: false,
                    ..
                },
            ) => {
                let disabled = self.disable_custom_curve();
                Err(match disabled {
                    Ok(()) => format!("{error}; Custom was disabled and its mappings were reset"),
                    Err(config_error) => format!(
                        "{error}; CRITICAL: Custom could not be disabled after the unverified rollback ({config_error})"
                    ),
                })
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn handle_profile_unlocked(
        &self,
        profile: PowerProfile,
        reason: &str,
        deduplicate: bool,
    ) -> Result<(), String> {
        if deduplicate {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?;
            if state.last_profile.is_some_and(|(previous, at)| {
                previous == profile && at.elapsed() < Duration::from_secs(1)
            }) {
                return Ok(());
            }
            state.last_profile = Some((profile, Instant::now()));
        }

        let mode = self.mapped_mode(profile)?;
        let custom_curve = if mode == FanMode::Custom {
            Some(self.snapshot()?.2.custom_curve.ok_or_else(|| {
                "Custom mapping requires a stored, validated custom curve".to_owned()
            })?)
        } else {
            None
        };

        if self.snapshot()?.0 == DaemonMode::Shadow {
            eprintln!("aorusd shadow: {reason}: {profile} -> {mode}; no hardware write");
            self.clear_error();
            return Ok(());
        }

        self.require_write_mode()?;

        let hardware = self.hardware()?;
        if let Some(curve) = custom_curve {
            // Never merely select Custom: rewrite and verify its stored curve
            // first so firmware resets cannot activate stale/partial points.
            self.apply_stored_curve(&curve)
                .map_err(|error| format!("reapply {profile} -> custom: {error}"))?;
        } else {
            hardware
                .reselect_fan_mode(mode)
                .map_err(|error| format!("reapply {profile} -> {mode}: {error}"))?;
        }
        eprintln!("aorusd: {reason}: applied {profile} -> {mode}");
        self.clear_error();
        Ok(())
    }

    fn handle_profile(
        &self,
        profile: PowerProfile,
        reason: &str,
        deduplicate: bool,
    ) -> Result<(), String> {
        let _guard = self
            .mutations
            .lock()
            .map_err(|_| "daemon mutation lock poisoned".to_owned())?;
        self.handle_profile_unlocked(profile, reason, deduplicate)
    }

    fn reapply_current(&self, reason: &str) -> Result<(), String> {
        self.handle_profile(profile::current_profile()?, reason, false)
    }

    fn reapply_current_unlocked(&self, reason: &str) -> Result<(), String> {
        self.handle_profile_unlocked(profile::current_profile()?, reason, false)
    }

    fn mutate<T>(
        &self,
        header: &Header<'_>,
        operation: impl FnOnce(&Self) -> Result<T, String>,
    ) -> zbus::fdo::Result<T> {
        // Gate before polkit so the safe default never prompts or touches hardware.
        self.require_write_mode()
            .map_err(zbus::fdo::Error::Failed)?;
        authorize(header)?;
        let _guard = self
            .mutations
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("daemon mutation lock poisoned".to_owned()))?;
        // Recheck after a potentially interactive polkit round-trip. The
        // write-mode service also Conflicts= with Python at the systemd layer.
        self.require_write_mode()
            .map_err(zbus::fdo::Error::Failed)?;
        operation(self).map_err(|error| zbus::fdo::Error::Failed(self.remember_error(error)))
    }

    fn replace_config(&self, config: Config) -> Result<(), String> {
        config
            .save_atomic(self.config_path()?)
            .map_err(|error| error.to_string())?;
        self.state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?
            .config = config;
        Ok(())
    }

    fn set_mappings(&self, requested: HashMap<String, u8>) -> Result<Config, String> {
        let previous = self.snapshot()?.2;
        let mut config = previous.clone();
        for (profile_name, mode_value) in requested {
            let profile = profile_name
                .parse::<PowerProfile>()
                .map_err(|_| format!("unsupported power-profile mapping key {profile_name:?}"))?;
            let mode = FanMode::try_from(mode_value).map_err(|error| error.to_string())?;
            if !mode.is_profile() {
                return Err(format!(
                    "mapping {profile} -> {mode} is not a writable profile mode"
                ));
            }
            config
                .set_mapping(profile, mode)
                .map_err(|error| error.to_string())?;
        }

        self.replace_config(config)?;
        Ok(previous)
    }

    fn capture_current_curve(&self) -> Result<FanCurve, String> {
        let curve = self
            .hardware()?
            .read_curve()
            .map_err(|error| self.forget_hardware(format!("hardware curve capture: {error}")))?;
        let mut config = self.snapshot()?.2;
        config
            .set_custom_curve(curve.clone())
            .map_err(|error| error.to_string())?;
        config
            .save_atomic(self.config_path()?)
            .map_err(|error| error.to_string())?;
        self.state
            .lock()
            .map_err(|_| "daemon state lock poisoned".to_owned())?
            .config = config;
        self.remember_curve(curve.clone());
        self.clear_error();
        Ok(curve)
    }
}

#[interface(name = "io.github.aoruslinux.Control1")]
impl AorusControl {
    fn get_status(&self) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        let _ = self.hardware();
        let (mode, hardware, config, last_error) =
            self.snapshot().map_err(zbus::fdo::Error::Failed)?;
        let mut hardware_error = None;
        let mut status = match hardware.as_ref() {
            Some(hardware) => match hardware.status(mode) {
                Ok(status) => {
                    self.clear_hardware_error();
                    status
                }
                Err(error) => {
                    let error = format!("hardware status: {error}");
                    hardware_error = Some(error.clone());
                    if let Ok(mut state) = self.state.lock() {
                        state.hardware = None;
                        state.curve_cache = None;
                        state.last_error = Some(error);
                    }
                    Status {
                        daemon_mode: mode,
                        driver_available: false,
                        ..Status::default()
                    }
                }
            },
            None => Status {
                daemon_mode: mode,
                driver_available: false,
                ..Status::default()
            },
        };
        if status.last_error.is_none() {
            status.last_error = last_error.or(hardware_error);
        }
        (
            status.power_profile,
            status.graphics_mode,
            status.graphics_power,
        ) = profile::system_state();
        status.custom_curve_available = config.custom_curve.is_some();
        let mut values = status_dictionary(status);
        for (key, path) in [
            ("product_name", "/sys/class/dmi/id/product_name"),
            ("product_version", "/sys/class/dmi/id/product_version"),
            ("bios_version", "/sys/class/dmi/id/bios_version"),
            ("bios_date", "/sys/class/dmi/id/bios_date"),
            ("kernel_release", "/proc/sys/kernel/osrelease"),
            ("driver_module_version", "/sys/module/aorus_laptop/version"),
        ] {
            if let Ok(value) = std::fs::read_to_string(path) {
                put_string(&mut values, key, value.trim().to_owned());
            }
        }
        if let Some(hardware) = hardware {
            if let Some(path) = &hardware.paths().platform {
                put_string(&mut values, "platform_path", path.display().to_string());
            }
            if let Some(path) = &hardware.paths().hwmon {
                put_string(&mut values, "hwmon_path", path.display().to_string());
            }
        }
        let native_keys = native_keys::status();
        put(
            &mut values,
            "native_fn_keys_supported",
            native_keys.supported,
        );
        put(&mut values, "native_fn_keys_enabled", native_keys.enabled);
        put(&mut values, "native_fn_keys_active", native_keys.active);
        Ok(values)
    }

    fn get_profile_mappings(&self) -> zbus::fdo::Result<HashMap<String, u8>> {
        let config = self.snapshot().map_err(zbus::fdo::Error::Failed)?.2;
        Ok(config
            .profile_mappings
            .into_iter()
            .map(|(profile, mode)| (profile.to_string(), mode.as_u8()))
            .collect())
    }

    fn get_fan_curve(&self) -> zbus::fdo::Result<Vec<(u8, u8)>> {
        let curve = if let Some(curve) = self.cached_curve() {
            curve
        } else {
            let (mode, _, config, _) = self.snapshot().map_err(zbus::fdo::Error::Failed)?;
            let curve = if mode == DaemonMode::Shadow {
                config.custom_curve.ok_or_else(|| {
                    zbus::fdo::Error::Failed(
                        "fan-curve selector writes are disabled in shadow mode; no stored validated curve is available"
                            .to_owned(),
                    )
                })?
            } else {
                self.hardware()
                    .map_err(zbus::fdo::Error::Failed)?
                    .read_curve()
                    .map_err(|error| {
                        zbus::fdo::Error::Failed(
                            self.forget_hardware(format!("hardware curve read: {error}")),
                        )
                    })?
            };
            self.remember_curve(curve.clone());
            curve
        };
        Ok(curve
            .points()
            .iter()
            .map(|point| (point.temperature, point.raw_speed))
            .collect())
    }

    fn set_power_profile(
        &self,
        value: &str,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        // This is safe in shadow mode: System76 remains the CPU-policy authority,
        // and the active Python service performs the corresponding fan-profile write.
        authorize(&header)?;
        let _guard = self
            .mutations
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("daemon mutation lock poisoned".to_owned()))?;
        (|| {
            let profile = value
                .parse::<PowerProfile>()
                .map_err(|_| format!("unsupported power profile {value:?}"))?;
            profile::set_profile(profile)?;
            self.handle_profile_unlocked(profile, "SetPowerProfile", true)
        })()
        .map_err(|error| zbus::fdo::Error::Failed(self.remember_error(error)))
    }

    fn set_fan_mode(&self, value: u8, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        self.mutate(&header, |control| {
            let mode = FanMode::try_from(value).map_err(|error| error.to_string())?;
            if !mode.is_profile() {
                return Err(format!("fan mode {mode} is not a writable profile mode"));
            }
            let hardware = control.hardware()?;
            if mode == FanMode::Custom {
                let curve = control.snapshot()?.2.custom_curve.ok_or_else(|| {
                    "Custom mode requires a stored, validated custom curve".to_owned()
                })?;
                control.apply_stored_curve(&curve)?;
            } else {
                hardware
                    .reselect_fan_mode(mode)
                    .map_err(|error| error.to_string())?;
            }
            control.clear_error();
            eprintln!("aorusd: SetFanMode applied {mode}");
            Ok(())
        })
    }

    fn reapply_fan_profile(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        self.mutate(&header, |control| {
            control.reapply_current_unlocked("manual reapply")
        })
    }

    fn set_fan_curve(
        &self,
        points: Vec<(u8, u8)>,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.mutate(&header, |control| {
            let curve = FanCurve::new(
                points
                    .into_iter()
                    .map(|(temperature, raw_speed)| FanPoint::new(temperature, raw_speed))
                    .collect(),
            )
            .map_err(|error| error.to_string())?;
            let previous_config = control.snapshot()?.2;
            let mut next_config = previous_config.clone();
            next_config
                .set_custom_curve(curve.clone())
                .map_err(|error| error.to_string())?;
            let path = control.config_path()?;
            next_config
                .save_atomic(&path)
                .map_err(|error| error.to_string())?;
            if let Err(error) = control
                .hardware()?
                .apply_curve(next_config.custom_curve.as_ref().expect("curve was just stored"))
            {
                let rollback_unverified = matches!(
                    &error,
                    HardwareError::Transaction {
                        rollback_verified: false,
                        ..
                    }
                );
                let mut restored_config = previous_config;
                if rollback_unverified {
                    restored_config.invalidate_custom_curve();
                }
                let restore = restored_config.save_atomic(&path);
                if rollback_unverified
                    && let Ok(mut state) = control.state.lock()
                {
                    state.config = restored_config;
                    state.curve_cache = None;
                }
                return Err(match restore {
                    Ok(()) if rollback_unverified => format!(
                        "curve apply and rollback verification failed; Custom was disabled and its mappings were reset: {error}"
                    ),
                    Ok(()) => {
                        format!("curve apply failed and prior configuration was restored: {error}")
                    }
                    Err(config_error) => format!(
                        "curve apply failed ({error}); CRITICAL: prior configuration could not be restored ({config_error})"
                    ),
                });
            }
            control
                .state
                .lock()
                .map_err(|_| "daemon state lock poisoned".to_owned())?
                .config = next_config;
            control.remember_curve(curve);
            control.clear_error();
            eprintln!("aorusd: SetFanCurve applied and verified 15 points; selected custom");
            Ok(())
        })
    }

    fn capture_fan_curve(
        &self,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<Vec<(u8, u8)>> {
        self.mutate(&header, |control| {
            let curve = control.capture_current_curve()?;
            eprintln!("aorusd: CaptureFanCurve stored 15 points without changing fan profile");
            Ok(curve
                .points()
                .iter()
                .map(|point| (point.temperature, point.raw_speed))
                .collect())
        })
    }

    fn set_profile_mappings(
        &self,
        mappings: HashMap<String, u8>,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.mutate(&header, |control| {
            let previous = control.set_mappings(mappings)?;
            if let Err(error) = control.reapply_current_unlocked("profile mapping update") {
                let rollback = control
                    .replace_config(previous)
                    .and_then(|()| control.reapply_current_unlocked("profile mapping rollback"));
                return Err(match rollback {
                    Ok(()) => format!(
                        "profile mapping update failed; prior mappings and profile were restored: {error}"
                    ),
                    Err(rollback_error) => format!(
                        "profile mapping update failed ({error}); CRITICAL: prior mapping/profile restoration failed ({rollback_error})"
                    ),
                });
            }
            control.clear_error();
            eprintln!("aorusd: SetProfileMappings saved mappings and reapplied the active profile");
            Ok(())
        })
    }

    fn set_charge_mode(
        &self,
        mode: u8,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.mutate(&header, |control| {
            control
                .hardware()?
                .set_charge_mode(mode)
                .map_err(|error| error.to_string())?;
            control.clear_error();
            eprintln!("aorusd: SetChargeMode applied and verified {mode}");
            Ok(())
        })
    }

    fn set_charge_limit(
        &self,
        limit: u8,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.mutate(&header, |control| {
            control
                .hardware()?
                .set_charge_limit(limit)
                .map_err(|error| error.to_string())?;
            control.clear_error();
            eprintln!("aorusd: SetChargeLimit applied and verified {limit}%");
            Ok(())
        })
    }

    fn set_gpu_boost(
        &self,
        boost: u8,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.mutate(&header, |control| {
            let hardware = control.hardware()?;
            if !hardware.capabilities().gpu_boost_values.contains(&boost) {
                return Err(format!(
                    "GPU boost value {boost} has not been verified on this model"
                ));
            }
            hardware
                .set_gpu_boost(boost)
                .map_err(|error| error.to_string())?;
            control.clear_error();
            eprintln!("aorusd: SetGpuBoost applied and verified {boost}");
            Ok(())
        })
    }

    fn set_native_fn_keys_enabled(
        &self,
        enabled: bool,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        self.mutate(&header, |_control| native_keys::set_enabled(enabled))
    }
}

fn status_dictionary(status: Status) -> HashMap<String, OwnedValue> {
    let mut values = HashMap::new();
    put_string(&mut values, "daemon_mode", status.daemon_mode.to_string());
    put(&mut values, "driver_available", status.driver_available);
    if let Some(profile) = status.power_profile {
        put_string(&mut values, "power_profile", profile.to_string());
    }
    if let Some(mode) = status.fan_mode {
        put(&mut values, "fan_mode", mode.as_u8());
    }
    for (key, temperature) in [
        ("temp1_millicelsius", status.temp1_millicelsius),
        ("temp2_millicelsius", status.temp2_millicelsius),
        ("temp3_millicelsius", status.temp3_millicelsius),
    ] {
        if let Some(temperature) = temperature {
            put(&mut values, key, temperature);
        }
    }
    for (key, rpm) in [
        ("fan1_rpm", status.fan1_rpm),
        ("fan2_rpm", status.fan2_rpm),
        ("fan3_rpm", status.fan3_rpm),
        ("fan4_rpm", status.fan4_rpm),
    ] {
        if let Some(rpm) = rpm.filter(|rpm| *rpm > 0) {
            put(&mut values, key, rpm);
        }
    }
    if let Some(value) = status.charge_mode {
        put(&mut values, "charge_mode", value);
    }
    if let Some(value) = status.charge_limit_percent {
        put(&mut values, "charge_limit_percent", value);
    }
    if let Some(value) = status.battery_cycles {
        put(&mut values, "battery_cycles", value);
    }
    if let Some(value) = status.gpu_boost {
        put(&mut values, "gpu_boost", value);
    }
    if let Some(value) = status.usb_charge_s3 {
        put(&mut values, "usb_charge_s3", value);
    }
    if let Some(value) = status.usb_charge_s4 {
        put(&mut values, "usb_charge_s4", value);
    }
    if let Some(value) = status.graphics_mode {
        put_string(&mut values, "graphics_mode", value);
    }
    if let Some(value) = status.graphics_power {
        put(&mut values, "graphics_power", value);
    }
    put(
        &mut values,
        "custom_curve_available",
        status.custom_curve_available,
    );
    if let Some(value) = status.last_error {
        put_string(&mut values, "last_error", value);
    }
    put(
        &mut values,
        "cap_fan_curve_points",
        status.capabilities.curve_points.unwrap_or(0),
    );
    put_array(
        &mut values,
        "cap_fan_modes",
        status
            .capabilities
            .fan_modes
            .into_iter()
            .map(FanMode::as_u8)
            .collect(),
    );
    put(
        &mut values,
        "cap_charge_mode",
        status.capabilities.charge_mode,
    );
    put(
        &mut values,
        "cap_charge_limit",
        status.capabilities.charge_limit,
    );
    put(
        &mut values,
        "cap_usb_charge_s3",
        status.capabilities.usb_charge_s3,
    );
    put(
        &mut values,
        "cap_usb_charge_s4",
        status.capabilities.usb_charge_s4,
    );
    put_array(
        &mut values,
        "cap_gpu_boost_values",
        status.capabilities.gpu_boost_values,
    );
    values
}

fn put<T: Into<OwnedValue>>(values: &mut HashMap<String, OwnedValue>, key: &str, value: T) {
    values.insert(key.to_owned(), value.into());
}

fn put_string(values: &mut HashMap<String, OwnedValue>, key: &str, value: String) {
    values.insert(
        key.to_owned(),
        OwnedValue::from(zbus::zvariant::Str::from(value)),
    );
}

fn put_array(values: &mut HashMap<String, OwnedValue>, key: &str, value: Vec<u8>) {
    let value = Value::from(Array::from(value));
    values.insert(
        key.to_owned(),
        OwnedValue::try_from(value).expect("byte arrays always have an owned D-Bus representation"),
    );
}

fn discover_hardware() -> Result<Hardware, crate::hardware::HardwareError> {
    match (
        std::env::var_os("AORUS_CONTROL_PLATFORM_ROOT"),
        std::env::var_os("AORUS_CONTROL_HWMON_ROOT"),
    ) {
        (Some(platform), Some(hwmon)) => Hardware::discover_at(platform, hwmon),
        (Some(platform), None) => Hardware::discover_at(platform, "/sys/class/hwmon"),
        (None, Some(hwmon)) => Hardware::discover_at("/sys/devices/platform", hwmon),
        (None, None) => Hardware::discover(),
    }
}

pub fn python_service_active() -> Result<bool, String> {
    Command::new("systemctl")
        .args(["is-active", "--quiet", "aorus-power-profile-sync.service"])
        .status()
        .map(|status| status.success())
        .map_err(|error| format!("cannot verify Python service ownership with systemctl: {error}"))
}

fn authorize(header: &Header<'_>) -> zbus::fdo::Result<()> {
    let sender = header
        .sender()
        .ok_or_else(|| zbus::fdo::Error::AccessDenied("mutation has no D-Bus sender".to_owned()))?;
    let connection =
        system_connection().map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
    let authority = Proxy::new(
        &connection,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;

    // PolicyKit's system-bus-name subject uses the unique bus name supplied by
    // the authenticated message header, rather than an untrusted caller PID.
    let mut subject_details = HashMap::new();
    subject_details.insert(
        "name".to_owned(),
        OwnedValue::from(zbus::zvariant::Str::from(sender.to_string())),
    );
    let subject = ("system-bus-name", subject_details);
    let details: HashMap<String, String> = HashMap::new();
    let (allowed, _challenge, _details): (bool, bool, HashMap<String, String>) = authority
        .call(
            "CheckAuthorization",
            &(subject, POLKIT_ACTION, details, 1u32, ""),
        )
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
    if allowed {
        Ok(())
    } else {
        Err(zbus::fdo::Error::AccessDenied(format!(
            "caller {sender} is not authorized for {POLKIT_ACTION}"
        )))
    }
}

pub fn spawn_background_workers(control: AorusControl) {
    if let Err(error) = control.reapply_current("startup") {
        control.remember_error(error);
    }
    spawn("system76-profile", {
        let control = control.clone();
        move || system76_worker(control)
    });
    spawn("upower-profile", {
        let control = control.clone();
        move || upower_profile_worker(control)
    });
    for path in upower_device_paths() {
        let control = control.clone();
        spawn("upower-device", move || upower_device_worker(control, path));
    }
    let logind_control = control.clone();
    spawn("logind-resume", move || logind_worker(logind_control));
    spawn("fan-watchdog", move || watchdog_worker(control));
}

fn spawn(name: &'static str, worker: impl FnOnce() + Send + 'static) {
    let _ = thread::Builder::new().name(name.to_owned()).spawn(worker);
}

fn system76_worker(control: AorusControl) {
    loop {
        let Ok(connection) = system_connection() else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(proxy) = Proxy::new(
            &connection,
            "com.system76.PowerDaemon",
            "/com/system76/PowerDaemon",
            "com.system76.PowerDaemon",
        ) else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(signals) = proxy.receive_signal("PowerProfileSwitch") else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        for message in signals {
            if let Ok(value) = message.body().deserialize::<String>()
                && let Some(profile) = profile::normalize(&value)
                && let Err(error) =
                    control.handle_profile(profile, "System76 PowerProfileSwitch", true)
            {
                control.remember_error(error);
            }
        }
    }
}

fn upower_profile_worker(control: AorusControl) {
    loop {
        let Ok(connection) = system_connection() else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(proxy) = Proxy::new(
            &connection,
            "org.freedesktop.UPower.PowerProfiles",
            "/org/freedesktop/UPower/PowerProfiles",
            "org.freedesktop.DBus.Properties",
        ) else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(signals) = proxy.receive_signal("PropertiesChanged") else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        for message in signals {
            let Ok((interface, changed, _)) =
                message
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
            else {
                continue;
            };
            if interface == "org.freedesktop.UPower.PowerProfiles"
                && changed.contains_key("ActiveProfile")
            {
                thread::sleep(POWER_REAPPLY_DELAY);
                if let Ok(current) = profile::current_profile_with(&connection)
                    && let Err(error) =
                        control.handle_profile(current, "UPower ActiveProfile", true)
                {
                    control.remember_error(error);
                }
            }
        }
    }
}

fn upower_device_paths() -> Vec<String> {
    let result = (|| {
        let connection = system_connection().ok()?;
        let proxy = Proxy::new(
            &connection,
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower",
            "org.freedesktop.UPower",
        )
        .ok()?;
        proxy
            .call::<_, _, Vec<OwnedObjectPath>>("EnumerateDevices", &())
            .ok()
    })();
    result
        .unwrap_or_default()
        .into_iter()
        .map(|path| path.to_string())
        .filter(|path| path.contains("/battery_") || path.contains("/line_power_"))
        .collect()
}

fn upower_device_worker(control: AorusControl, path: String) {
    loop {
        let Ok(connection) = system_connection() else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(proxy) = Proxy::new(
            &connection,
            "org.freedesktop.UPower",
            path.as_str(),
            "org.freedesktop.DBus.Properties",
        ) else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(signals) = proxy.receive_signal("PropertiesChanged") else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        for message in signals {
            let Ok((_interface, changed, _invalidated)) =
                message
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
            else {
                continue;
            };
            if !changed.contains_key("Online") && !changed.contains_key("State") {
                continue;
            }
            thread::sleep(POWER_REAPPLY_DELAY);
            if let Ok(current) = profile::current_profile_with(&connection)
                && let Err(error) =
                    control.handle_profile(current, "UPower AC/battery change", true)
            {
                control.remember_error(error);
            }
        }
    }
}

fn logind_worker(control: AorusControl) {
    loop {
        let Ok(connection) = system_connection() else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(proxy) = Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        ) else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        let Ok(signals) = proxy.receive_signal("PrepareForSleep") else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        for message in signals {
            if let Ok(sleeping) = message.body().deserialize::<bool>()
                && !sleeping
            {
                thread::sleep(RESUME_REAPPLY_DELAY);
                if let Err(error) = control.reapply_current("resume") {
                    control.remember_error(error);
                }
            }
        }
    }
}

fn watchdog_worker(control: AorusControl) {
    loop {
        thread::sleep(WATCHDOG_INTERVAL);
        let Ok(profile) = profile::current_profile() else {
            continue;
        };
        let Ok(hardware) = control.hardware() else {
            continue;
        };
        let Ok(status) = hardware.status(
            control
                .snapshot()
                .map(|snapshot| snapshot.0)
                .unwrap_or(DaemonMode::Shadow),
        ) else {
            continue;
        };
        if status.fan_mode != control.mapped_mode(profile).ok()
            && let Err(error) = control.handle_profile(profile, "watchdog mismatch", false)
        {
            control.remember_error(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_status_omits_unsupported_values() {
        let status = Status {
            daemon_mode: DaemonMode::Shadow,
            driver_available: false,
            ..Status::default()
        };
        let values = status_dictionary(status);
        assert!(values.contains_key("daemon_mode"));
        assert!(!values.contains_key("fan1_rpm"));
        assert!(!values.contains_key("last_error"));
    }
}
