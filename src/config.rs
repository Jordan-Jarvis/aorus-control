//! Persistent daemon configuration.

use crate::{
    curve::{CurveError, FanCurve},
    model::{FanMode, ModelError, PowerProfile},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const CONFIG_VERSION: u32 = 1;

pub type ProfileMappings = BTreeMap<PowerProfile, FanMode>;

pub fn default_mappings() -> ProfileMappings {
    BTreeMap::from([
        (PowerProfile::Performance, FanMode::Gaming),
        (PowerProfile::Balanced, FanMode::Normal),
        (PowerProfile::Battery, FanMode::Silent),
    ])
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub version: u32,
    pub profile_mappings: ProfileMappings,
    pub custom_curve: Option<FanCurve>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            profile_mappings: default_mappings(),
            custom_curve: None,
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let file: FileConfig =
            toml::from_str(text).map_err(|error| ConfigError::Toml(error.to_string()))?;
        let mut profile_mappings = BTreeMap::new();
        for (profile, mode) in file.profile_mappings {
            let profile = profile
                .parse::<PowerProfile>()
                .map_err(ConfigError::Model)?;
            let mode = mode.parse::<FanMode>().map_err(ConfigError::Model)?;
            if profile_mappings.insert(profile, mode).is_some() {
                return Err(ConfigError::DuplicateMapping(profile));
            }
        }
        let custom_curve = file
            .custom_curve
            .map(|curve| {
                FanCurve::new(
                    curve
                        .points
                        .into_iter()
                        .map(|[temperature, raw_speed]| {
                            crate::curve::FanPoint::new(temperature, raw_speed)
                        })
                        .collect(),
                )
            })
            .transpose()
            .map_err(ConfigError::Curve)?;
        let config = Self {
            version: file.version,
            profile_mappings: with_default_mappings(profile_mappings),
            custom_curve,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        Self::parse(&fs::read_to_string(path).map_err(|source| ConfigError::Io {
            operation: "read",
            path: path.to_owned(),
            source,
        })?)
    }

    pub fn to_toml(&self) -> Result<String, ConfigError> {
        self.validate()?;
        let file = FileConfig {
            version: self.version,
            profile_mappings: self
                .profile_mappings
                .iter()
                .map(|(profile, mode)| (profile.to_string(), mode.to_string()))
                .collect(),
            custom_curve: self.custom_curve.as_ref().map(|curve| StoredCurve {
                points: curve
                    .points()
                    .iter()
                    .map(|point| [point.temperature, point.raw_speed])
                    .collect(),
            }),
        };
        toml::to_string_pretty(&file).map_err(|error| ConfigError::Toml(error.to_string()))
    }

    pub fn save_atomic(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let text = self.to_toml()?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = parent.join(format!(
            ".{}.{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("aorus-control"),
            stamp
        ));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|source| ConfigError::Io {
                    operation: "create temporary config",
                    path: temporary.clone(),
                    source,
                })?;
            file.write_all(text.as_bytes())
                .map_err(|source| ConfigError::Io {
                    operation: "write temporary config",
                    path: temporary.clone(),
                    source,
                })?;
            file.sync_all().map_err(|source| ConfigError::Io {
                operation: "sync temporary config",
                path: temporary.clone(),
                source,
            })?;
            fs::rename(&temporary, path).map_err(|source| ConfigError::Io {
                operation: "replace config",
                path: path.to_owned(),
                source,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != CONFIG_VERSION {
            return Err(ConfigError::Version(self.version));
        }
        for profile in PowerProfile::ALL {
            let mode = *self
                .profile_mappings
                .get(&profile)
                .ok_or(ConfigError::MissingMapping(profile))?;
            if !mode.is_profile() {
                return Err(ConfigError::InvalidMapping { profile, mode });
            }
            if mode == FanMode::Custom && self.custom_curve.is_none() {
                return Err(ConfigError::CustomCurveRequired(profile));
            }
        }
        if let Some(curve) = &self.custom_curve {
            curve.validate().map_err(ConfigError::Curve)?;
        }
        Ok(())
    }

    pub fn mapping(&self, profile: PowerProfile) -> FanMode {
        self.profile_mappings[&profile]
    }

    pub fn set_mapping(&mut self, profile: PowerProfile, mode: FanMode) -> Result<(), ConfigError> {
        if !mode.is_profile() {
            return Err(ConfigError::InvalidMapping { profile, mode });
        }
        if mode == FanMode::Custom && self.custom_curve.is_none() {
            return Err(ConfigError::CustomCurveRequired(profile));
        }
        self.profile_mappings.insert(profile, mode);
        Ok(())
    }

    pub fn set_custom_curve(&mut self, curve: FanCurve) -> Result<(), ConfigError> {
        curve.validate().map_err(ConfigError::Curve)?;
        self.custom_curve = Some(curve);
        Ok(())
    }

    /// Disable Custom after an unverified hardware rollback. Any Custom
    /// mappings return to their conservative firmware defaults.
    pub fn invalidate_custom_curve(&mut self) {
        self.custom_curve = None;
        for (profile, mode) in default_mappings() {
            if self.profile_mappings.get(&profile) == Some(&FanMode::Custom) {
                self.profile_mappings.insert(profile, mode);
            }
        }
    }
}

fn with_default_mappings(mut mappings: ProfileMappings) -> ProfileMappings {
    for (profile, mode) in default_mappings() {
        mappings.entry(profile).or_insert(mode);
    }
    mappings
}

#[derive(Debug)]
pub enum ConfigError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Toml(String),
    Version(u32),
    MissingMapping(PowerProfile),
    InvalidMapping {
        profile: PowerProfile,
        mode: FanMode,
    },
    CustomCurveRequired(PowerProfile),
    DuplicateMapping(PowerProfile),
    Curve(CurveError),
    Model(ModelError),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(f, "{operation} {}: {source}", path.display()),
            Self::Toml(error) => error.fmt(f),
            Self::Version(version) => write!(
                f,
                "unsupported config version {version}; expected {CONFIG_VERSION}"
            ),
            Self::MissingMapping(profile) => write!(f, "missing mapping for {profile}"),
            Self::InvalidMapping { profile, mode } => {
                write!(f, "invalid mapping {profile} -> {mode}")
            }
            Self::CustomCurveRequired(profile) => write!(
                f,
                "mapping {profile} -> custom requires a stored custom curve"
            ),
            Self::DuplicateMapping(profile) => write!(f, "duplicate mapping for {profile}"),
            Self::Curve(error) => error.fmt(f),
            Self::Model(error) => error.fmt(f),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct FileConfig {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    profile_mappings: BTreeMap<String, String>,
    #[serde(default)]
    custom_curve: Option<StoredCurve>,
}

#[derive(Debug, Deserialize, Serialize)]
struct StoredCurve {
    points: Vec<[u8; 2]>,
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{FAN_CURVE_POINTS, FanPoint};

    fn valid_curve() -> FanCurve {
        FanCurve::from_valid_array(std::array::from_fn(|index| {
            FanPoint::new(index as u8 * 5, index as u8 * 10)
        }))
    }

    #[test]
    fn defaults_match_firmware_profiles() {
        let config = Config::default();
        assert_eq!(config.mapping(PowerProfile::Performance), FanMode::Gaming);
        assert_eq!(config.mapping(PowerProfile::Balanced), FanMode::Normal);
        assert_eq!(config.mapping(PowerProfile::Battery), FanMode::Silent);
    }

    #[test]
    fn parse_normalizes_missing_mappings_and_rejects_bad_version() {
        let config =
            Config::parse("version = 1\n[profile_mappings]\nperformance = 'gaming'\n").unwrap();
        assert_eq!(config, Config::default());
        assert!(matches!(
            Config::parse("version = 2"),
            Err(ConfigError::Version(2))
        ));
    }

    #[test]
    fn custom_mapping_requires_curve_and_round_trips_atomically() {
        let mut config = Config::default();
        assert!(matches!(
            config.set_mapping(PowerProfile::Performance, FanMode::Custom),
            Err(ConfigError::CustomCurveRequired(_))
        ));
        config.set_custom_curve(valid_curve()).unwrap();
        config
            .set_mapping(PowerProfile::Performance, FanMode::Custom)
            .unwrap();
        let text = config.to_toml().unwrap();
        assert_eq!(Config::parse(&text).unwrap(), config);
        let path = std::env::temp_dir().join(format!(
            "aorus-config-{}-{}.toml",
            std::process::id(),
            FAN_CURVE_POINTS
        ));
        config.save_atomic(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn invalidating_curve_removes_custom_mappings() {
        let mut config = Config::default();
        config.set_custom_curve(valid_curve()).unwrap();
        config
            .set_mapping(PowerProfile::Balanced, FanMode::Custom)
            .unwrap();
        config.invalidate_custom_curve();
        assert!(config.custom_curve.is_none());
        assert_eq!(config.mapping(PowerProfile::Balanced), FanMode::Normal);
        config.validate().unwrap();
    }
}
