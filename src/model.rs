//! Shared status and capability models.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FanMode {
    Normal = 0,
    Silent = 1,
    Gaming = 2,
    Custom = 3,
    Auto = 4,
    Fixed = 5,
}

impl FanMode {
    pub const PROFILE_MODES: [Self; 4] = [Self::Normal, Self::Silent, Self::Gaming, Self::Custom];

    pub const fn is_profile(self) -> bool {
        matches!(
            self,
            Self::Normal | Self::Silent | Self::Gaming | Self::Custom
        )
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for FanMode {
    type Error = ModelError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Normal),
            1 => Ok(Self::Silent),
            2 => Ok(Self::Gaming),
            3 => Ok(Self::Custom),
            4 => Ok(Self::Auto),
            5 => Ok(Self::Fixed),
            value => Err(ModelError::InvalidFanMode(value)),
        }
    }
}

impl fmt::Display for FanMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Normal => "normal",
            Self::Silent => "silent",
            Self::Gaming => "gaming",
            Self::Custom => "custom",
            Self::Auto => "auto",
            Self::Fixed => "fixed",
        })
    }
}

impl FromStr for FanMode {
    type Err = ModelError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Ok(Self::Normal),
            "silent" => Ok(Self::Silent),
            "gaming" => Ok(Self::Gaming),
            "custom" => Ok(Self::Custom),
            "auto" => Ok(Self::Auto),
            "fixed" => Ok(Self::Fixed),
            _ => Err(ModelError::InvalidFanModeName(value.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerProfile {
    Performance,
    Balanced,
    Battery,
}

impl PowerProfile {
    pub const ALL: [Self; 3] = [Self::Performance, Self::Balanced, Self::Battery];
}

impl fmt::Display for PowerProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Performance => "performance",
            Self::Balanced => "balanced",
            Self::Battery => "battery",
        })
    }
}

impl FromStr for PowerProfile {
    type Err = ModelError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "performance" => Ok(Self::Performance),
            "balanced" => Ok(Self::Balanced),
            "battery" | "power-saver" | "powersaver" => Ok(Self::Battery),
            _ => Err(ModelError::InvalidPowerProfile(value.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonMode {
    #[default]
    Shadow,
    WriteEnabled,
}

impl fmt::Display for DaemonMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Shadow => "shadow",
            Self::WriteEnabled => "write-enabled",
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub fan_modes: Vec<FanMode>,
    pub curve_points: Option<u8>,
    pub charge_mode: bool,
    pub charge_limit: bool,
    pub gpu_boost_values: Vec<u8>,
    pub usb_charge_s3: bool,
    pub usb_charge_s4: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub daemon_mode: DaemonMode,
    pub driver_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_profile: Option<PowerProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_mode: Option<FanMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp1_millicelsius: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp2_millicelsius: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp3_millicelsius: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan1_rpm: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan2_rpm: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan3_rpm: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan4_rpm: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_mode: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_limit_percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_cycles: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_boost: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_charge_s3: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_charge_s4: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graphics_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graphics_power: Option<bool>,
    pub custom_curve_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelError {
    InvalidFanMode(u8),
    InvalidFanModeName(String),
    InvalidPowerProfile(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFanMode(value) => write!(f, "invalid fan mode {value}"),
            Self::InvalidFanModeName(value) => write!(f, "invalid fan mode '{value}'"),
            Self::InvalidPowerProfile(value) => write!(f, "invalid power profile '{value}'"),
        }
    }
}

impl std::error::Error for ModelError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_and_text_fan_modes_agree() {
        for (number, mode) in FanMode::PROFILE_MODES.into_iter().enumerate() {
            assert_eq!(FanMode::try_from(number as u8).unwrap(), mode);
            assert_eq!(mode.to_string().parse(), Ok(mode));
        }
    }

    #[test]
    fn power_profile_accepts_power_daemon_alias() {
        assert_eq!("power-saver".parse(), Ok(PowerProfile::Battery));
    }
}
