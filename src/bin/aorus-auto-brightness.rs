//! Opt-in, user-session ambient-brightness policy for COSMIC.
//!
//! This process only reads the standard IIO sysfs interface and uses the
//! COSMIC settings daemon on the session bus. It never writes sysfs.

use std::{
    env, fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use zbus::blocking::{Connection, Proxy, connection::Builder};

const IIO_ROOT: &str = "/sys/bus/iio/devices";
const IIO_NAME: &str = "aorus-ambient-light";
const IIO_LUX_FILE: &str = "in_illuminance_input";
const COSMIC_DESTINATION: &str = "com.system76.CosmicSettingsDaemon";
const COSMIC_PATH: &str = "/com/system76/CosmicSettingsDaemon";
const COSMIC_INTERFACE: &str = "com.system76.CosmicSettingsDaemon";
const DBUS_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    /// Lowest sensor value, in lux, mapped to min_brightness.
    min_lux: f64,
    /// Highest sensor value, in lux, mapped to max_brightness.
    max_lux: f64,
    /// Minimum display brightness as a fraction of MaxDisplayBrightness.
    min_brightness: f64,
    /// Maximum display brightness as a fraction of MaxDisplayBrightness.
    max_brightness: f64,
    /// Fraction of the display range a change must exceed before it is used.
    hysteresis: f64,
    /// Seconds the target must remain stable before a write.
    settle_seconds: u64,
    /// Seconds to pause after an external brightness change.
    override_seconds: u64,
    /// Seconds between sensor and display polls.
    poll_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_lux: 1.0,
            max_lux: 1_000.0,
            min_brightness: 0.10,
            max_brightness: 1.0,
            hysteresis: 0.03,
            settle_seconds: 3,
            override_seconds: 300,
            poll_seconds: 2,
        }
    }
}

impl Config {
    fn validate(&self) -> Result<(), String> {
        if !self.min_lux.is_finite() || !self.max_lux.is_finite() || self.min_lux < 0.0 {
            return Err("min_lux and max_lux must be finite and non-negative".into());
        }
        if self.max_lux <= self.min_lux || self.min_lux.ln_1p() >= self.max_lux.ln_1p() {
            return Err("max_lux must be greater than min_lux".into());
        }
        if !self.min_brightness.is_finite()
            || !self.max_brightness.is_finite()
            || !(0.0..=1.0).contains(&self.min_brightness)
            || !(0.0..=1.0).contains(&self.max_brightness)
            || self.max_brightness < self.min_brightness
        {
            return Err("brightness fractions must be finite, in 0..=1, and ordered".into());
        }
        if !self.hysteresis.is_finite() || !(0.0..=1.0).contains(&self.hysteresis) {
            return Err("hysteresis must be a fraction in 0..=1".into());
        }
        if !(1..=3_600).contains(&self.poll_seconds)
            || self.settle_seconds > 86_400
            || self.override_seconds > 86_400
        {
            return Err(
                "poll_seconds must be 1..=3600 and other durations must be <= 86400".into(),
            );
        }
        Ok(())
    }

    fn load() -> Result<Self, String> {
        let Some(path) = config_path() else {
            return Ok(Self::default());
        };
        match fs::read_to_string(&path) {
            Ok(text) => {
                let config = toml::from_str::<Self>(&text)
                    .map_err(|error| format!("parse {}: {error}", path.display()))?;
                config
                    .validate()
                    .map_err(|error| format!("validate {}: {error}", path.display()))?;
                eprintln!("aorus-auto-brightness: using {}", path.display());
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("read {}: {error}", path.display())),
        }
    }

    fn settle(&self) -> Duration {
        Duration::from_secs(self.settle_seconds)
    }

    fn override_duration(&self) -> Duration {
        Duration::from_secs(self.override_seconds)
    }

    fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_seconds)
    }
}

fn config_path() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })?;
    Some(base.join("aorus-control/auto-brightness.toml"))
}

fn find_iio_lux_file(root: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("read IIO devices {}: {error}", root.display()))?;
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("enumerate IIO devices: {error}"))?;
        let device = entry.path();
        let name = match fs::read_to_string(device.join("name")) {
            Ok(name) => name,
            Err(_) => continue,
        };
        if name.trim() == IIO_NAME {
            let lux_file = device.join(IIO_LUX_FILE);
            if lux_file.is_file() {
                matches.push(lux_file);
            }
        }
    }

    match matches.as_slice() {
        [lux_file] => Ok(lux_file.clone()),
        [] => Err(format!(
            "IIO device {IIO_NAME:?} with {IIO_LUX_FILE:?} is not available under {}",
            root.display()
        )),
        _ => Err(format!(
            "multiple IIO devices named {IIO_NAME:?} are available under {}; refusing to guess",
            root.display()
        )),
    }
}

fn read_lux_from(root: &Path) -> Result<f64, String> {
    let path = find_iio_lux_file(root)?;
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("read ambient light {}: {error}", path.display()))?;
    let lux = text
        .trim()
        .parse::<f64>()
        .map_err(|error| format!("parse ambient light {}: {error}", path.display()))?;
    if !lux.is_finite() || lux < 0.0 {
        return Err(format!(
            "ambient light {} is invalid: {lux}",
            path.display()
        ));
    }
    Ok(lux)
}

fn read_lux() -> Result<f64, String> {
    read_lux_from(Path::new(IIO_ROOT))
}

fn map_lux_to_brightness(lux: f64, max_brightness: i32, config: &Config) -> i32 {
    let low = config.min_lux.ln_1p();
    let high = config.max_lux.ln_1p();
    let position =
        ((lux.clamp(config.min_lux, config.max_lux).ln_1p() - low) / (high - low)).clamp(0.0, 1.0);
    let fraction =
        config.min_brightness + (config.max_brightness - config.min_brightness) * position;
    (fraction * f64::from(max_brightness))
        .round()
        .clamp(0.0, f64::from(max_brightness)) as i32
}

#[derive(Debug, Default)]
struct PolicyState {
    last_observed: Option<i32>,
    expected_write: Option<(i32, bool)>,
    override_until: Option<Duration>,
    pending: Option<(i32, Duration)>,
}

impl PolicyState {
    fn step(
        &mut self,
        now: Duration,
        lux: f64,
        current: i32,
        maximum: i32,
        config: &Config,
    ) -> Option<i32> {
        let external_change = match self.expected_write.take() {
            Some((expected, _)) if expected == current => false,
            // A successful desktop write can be visible on the next poll. If
            // it is not visible yet, retain the expectation rather than
            // mistaking a stale read for a manual override.
            Some((expected, false)) if self.last_observed == Some(current) => {
                self.expected_write = Some((expected, true));
                false
            }
            // Give one stale poll to the asynchronous desktop setter. If the
            // value still did not change, retry normally; only a different
            // value is evidence of a manual change.
            Some((_, true)) if self.last_observed == Some(current) => false,
            Some(_) => true,
            None => self
                .last_observed
                .is_some_and(|previous| previous != current),
        };
        if external_change {
            self.override_until = Some(now.saturating_add(config.override_duration()));
            self.pending = None;
        }
        self.last_observed = Some(current);

        if self.override_until.is_some_and(|until| now < until) {
            return None;
        }
        self.override_until = None;

        let target = map_lux_to_brightness(lux, maximum, config);
        let threshold = (f64::from(maximum) * config.hysteresis).round() as i32;
        if (target - current).abs() <= threshold {
            self.pending = None;
            return None;
        }

        match self.pending {
            // Sensor noise smaller than the brightness hysteresis must not
            // restart settling on every sample. Keep the candidate current
            // while preserving the time at which it first became stable.
            Some((pending, since)) if (pending - target).abs() <= threshold => {
                self.pending = Some((target, since));
                if now.saturating_sub(since) >= config.settle() {
                    self.pending = None;
                    Some(target)
                } else {
                    None
                }
            }
            _ => {
                self.pending = Some((target, now));
                if config.settle().is_zero() {
                    self.pending = None;
                    Some(target)
                } else {
                    None
                }
            }
        }
    }

    fn record_write(&mut self, value: i32) {
        self.expected_write = Some((value, false));
    }

    fn reset_pending(&mut self) {
        self.pending = None;
    }
}

fn cosmic_proxy(connection: &Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(
        connection,
        COSMIC_DESTINATION,
        COSMIC_PATH,
        COSMIC_INTERFACE,
    )
    .map_err(|error| format!("create COSMIC settings proxy: {error}"))
}

struct PollError {
    message: String,
    reconnect: bool,
}

fn poll_once(
    state: &mut PolicyState,
    started: Instant,
    config: &Config,
    connection: &Connection,
) -> Result<Option<i32>, PollError> {
    let lux = read_lux().map_err(|message| PollError {
        message,
        reconnect: false,
    })?;
    let proxy = cosmic_proxy(connection).map_err(|message| PollError {
        message,
        reconnect: true,
    })?;
    let maximum = proxy
        .get_property::<i32>("MaxDisplayBrightness")
        .map_err(|error| PollError {
            message: format!("read MaxDisplayBrightness: {error}"),
            reconnect: true,
        })?;
    let current = proxy
        .get_property::<i32>("DisplayBrightness")
        .map_err(|error| PollError {
            message: format!("read DisplayBrightness: {error}"),
            reconnect: true,
        })?;
    if maximum <= 0 || !(0..=maximum).contains(&current) {
        return Err(PollError {
            message: format!(
                "COSMIC reported invalid brightness range: current={current}, maximum={maximum}"
            ),
            reconnect: false,
        });
    }

    let now = started.elapsed();
    let Some(target) = state.step(now, lux, current, maximum, config) else {
        return Ok(None);
    };
    proxy
        .set_property("DisplayBrightness", target)
        .map_err(|error| PollError {
            message: format!("write DisplayBrightness={target}: {error}"),
            reconnect: true,
        })?;
    state.record_write(target);
    Ok(Some(target))
}

fn run(config: Config) -> ! {
    let started = Instant::now();
    let mut state = PolicyState::default();
    let mut last_error = None::<String>;
    let mut connection = None;
    loop {
        if connection.is_none() {
            connection = Builder::session()
                .and_then(|builder| builder.method_timeout(DBUS_TIMEOUT).build())
                .map_err(|error| format!("connect to the session bus: {error}"))
                .map_err(|error| {
                    if last_error.as_deref() != Some(&error) {
                        eprintln!("aorus-auto-brightness: {error}; retrying");
                        last_error = Some(error);
                    }
                })
                .ok();
        }
        let result = connection
            .as_ref()
            .map(|connection| poll_once(&mut state, started, &config, connection));
        match result {
            Some(Ok(Some(target))) => {
                eprintln!("aorus-auto-brightness: set display brightness to {target}");
                last_error = None;
            }
            Some(Ok(None)) => {
                if last_error.take().is_some() {
                    eprintln!("aorus-auto-brightness: sensor and COSMIC settings recovered");
                }
            }
            Some(Err(error)) => {
                // A settling interval only counts consecutive valid samples;
                // a sensor or desktop outage must not age a pending target.
                state.reset_pending();
                if last_error.as_deref() != Some(&error.message) {
                    eprintln!("aorus-auto-brightness: {}; retrying", error.message);
                    last_error = Some(error.message);
                }
                if error.reconnect {
                    connection = None;
                }
            }
            None => state.reset_pending(),
        }
        thread::sleep(config.poll_interval());
    }
}

fn main() {
    if env::args().any(|argument| argument == "--help" || argument == "-h") {
        println!(
            "Usage: aorus-auto-brightness\n\nReads {IIO_NAME}'s {IIO_LUX_FILE} and adjusts COSMIC display brightness.\nOptional config: $XDG_CONFIG_HOME/aorus-control/auto-brightness.toml\n"
        );
        return;
    }
    let config = match Config::load() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("aorus-auto-brightness: {error}; refusing to start");
            std::process::exit(2);
        }
    };
    run(config);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config {
            min_lux: 1.0,
            max_lux: 1_000.0,
            min_brightness: 0.1,
            max_brightness: 0.9,
            hysteresis: 0.0,
            settle_seconds: 3,
            override_seconds: 10,
            poll_seconds: 2,
        }
    }

    #[test]
    fn logarithmic_mapping_is_bounded_and_monotonic() {
        let config = config();
        let dark = map_lux_to_brightness(0.0, 400, &config);
        let middle = map_lux_to_brightness(31.6, 400, &config);
        let bright = map_lux_to_brightness(10_000.0, 400, &config);
        assert_eq!(dark, 40);
        assert!(dark < middle && middle < bright);
        assert_eq!(bright, 360);
    }

    #[test]
    fn settle_interval_prevents_immediate_write() {
        let config = config();
        let mut state = PolicyState::default();
        assert_eq!(state.step(Duration::ZERO, 1_000.0, 40, 400, &config), None);
        assert_eq!(
            state.step(Duration::from_secs(2), 1_000.0, 40, 400, &config),
            None
        );
        assert_eq!(
            state.step(Duration::from_secs(3), 1_000.0, 40, 400, &config),
            Some(360)
        );
    }

    #[test]
    fn manual_change_pauses_until_override_expires() {
        let config = config();
        let mut state = PolicyState::default();
        assert_eq!(state.step(Duration::ZERO, 1_000.0, 40, 400, &config), None);
        assert_eq!(
            state.step(Duration::from_secs(1), 1_000.0, 100, 400, &config),
            None
        );
        assert_eq!(
            state.step(Duration::from_secs(10), 1_000.0, 100, 400, &config),
            None
        );
        assert_eq!(
            state.step(Duration::from_secs(11), 1_000.0, 100, 400, &config),
            None
        );
        assert_eq!(
            state.step(Duration::from_secs(14), 1_000.0, 100, 400, &config),
            Some(360)
        );
    }

    #[test]
    fn own_write_is_not_mistaken_for_manual_change() {
        let mut config = config();
        config.settle_seconds = 0;
        let mut state = PolicyState::default();
        assert_eq!(
            state.step(Duration::ZERO, 1_000.0, 40, 400, &config),
            Some(360)
        );
        state.record_write(360);
        assert_eq!(
            state.step(Duration::from_secs(1), 1_000.0, 360, 400, &config),
            None
        );
        assert_eq!(state.override_until, None);
    }

    #[test]
    fn delayed_own_write_is_not_mistaken_for_manual_change() {
        let mut config = config();
        config.settle_seconds = 0;
        let mut state = PolicyState::default();
        assert_eq!(
            state.step(Duration::ZERO, 1_000.0, 40, 400, &config),
            Some(360)
        );
        state.record_write(360);

        assert_eq!(
            state.step(Duration::from_secs(1), 1_000.0, 40, 400, &config),
            Some(360)
        );
        assert_eq!(state.override_until, None);
        assert_eq!(
            state.step(Duration::from_secs(2), 1_000.0, 360, 400, &config),
            None
        );
    }

    #[test]
    fn invalid_sample_cannot_complete_an_old_settle_interval() {
        let config = config();
        let mut state = PolicyState::default();
        assert_eq!(state.step(Duration::ZERO, 1_000.0, 40, 400, &config), None);
        state.reset_pending();
        assert_eq!(
            state.step(Duration::from_secs(3), 1_000.0, 40, 400, &config),
            None
        );
        assert_eq!(
            state.step(Duration::from_secs(6), 1_000.0, 40, 400, &config),
            Some(360)
        );
    }

    #[test]
    fn sensor_noise_does_not_restart_settling() {
        let mut config = config();
        config.hysteresis = 0.03;
        let mut state = PolicyState::default();
        assert_eq!(state.step(Duration::ZERO, 1_000.0, 40, 400, &config), None);

        let noisy_target = map_lux_to_brightness(900.0, 400, &config);
        assert!((noisy_target - 360).abs() <= 12);
        assert_eq!(
            state.step(Duration::from_secs(2), 900.0, 40, 400, &config),
            None
        );
        assert_eq!(
            state.step(Duration::from_secs(3), 900.0, 40, 400, &config),
            Some(noisy_target)
        );
    }

    #[test]
    fn discovers_and_parses_only_the_named_iio_sensor() {
        let root = std::env::temp_dir().join(format!(
            "aorus-auto-brightness-test-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let device = root.join("iio:device0");
        fs::create_dir_all(&device).unwrap();
        fs::write(device.join("name"), format!("{IIO_NAME}\n")).unwrap();
        fs::write(device.join(IIO_LUX_FILE), "123.5\n").unwrap();

        assert_eq!(read_lux_from(&root).unwrap(), 123.5);
        fs::write(device.join(IIO_LUX_FILE), "not-a-lux-value\n").unwrap();
        assert!(
            read_lux_from(&root)
                .unwrap_err()
                .contains("parse ambient light")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_ambiguous_or_incomplete_iio_discovery() {
        let root = std::env::temp_dir().join(format!(
            "aorus-auto-brightness-discovery-test-{}",
            std::process::id()
        ));
        let first = root.join("iio:device0");
        let second = root.join("iio:device1");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("name"), IIO_NAME).unwrap();
        fs::write(first.join(IIO_LUX_FILE), "1").unwrap();
        fs::write(second.join("name"), IIO_NAME).unwrap();
        fs::write(second.join(IIO_LUX_FILE), "2").unwrap();
        assert!(
            find_iio_lux_file(&root)
                .unwrap_err()
                .contains("multiple IIO devices")
        );

        fs::remove_file(first.join(IIO_LUX_FILE)).unwrap();
        fs::remove_file(second.join(IIO_LUX_FILE)).unwrap();
        assert!(
            find_iio_lux_file(&root)
                .unwrap_err()
                .contains("is not available")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_validation_rejects_unsafe_values() {
        let mut settings = config();
        assert!(settings.validate().is_ok());

        settings.max_lux = settings.min_lux;
        assert!(settings.validate().is_err());
        settings = config();
        settings.min_brightness = -0.01;
        assert!(settings.validate().is_err());
        settings = config();
        settings.hysteresis = f64::NAN;
        assert!(settings.validate().is_err());
        settings = config();
        settings.poll_seconds = 0;
        assert!(settings.validate().is_err());
    }
}
