//! Typed per-user mappings for the laptop's physical Fn buttons.

use std::{
    collections::BTreeMap,
    env, fmt, fs,
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PhysicalButtonId {
    BrightnessDown,
    BrightnessUp,
    Fan,
    Sleep,
    Wifi,
    Display,
    SquareX,
    TouchpadLock,
    Ai,
}

impl PhysicalButtonId {
    pub const ALL: [Self; 9] = [
        Self::BrightnessDown,
        Self::BrightnessUp,
        Self::Fan,
        Self::Sleep,
        Self::Wifi,
        Self::Display,
        Self::SquareX,
        Self::TouchpadLock,
        Self::Ai,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::BrightnessDown => "brightness-down",
            Self::BrightnessUp => "brightness-up",
            Self::Fan => "fan",
            Self::Sleep => "sleep",
            Self::Wifi => "wifi",
            Self::Display => "display",
            Self::SquareX => "square-x",
            Self::TouchpadLock => "touchpad-lock",
            Self::Ai => "ai",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::BrightnessDown => "Brightness down",
            Self::BrightnessUp => "Brightness up",
            Self::Fan => "Fan",
            Self::Sleep => "Sleep / Zz",
            Self::Wifi => "Wi-Fi",
            Self::Display => "Display / LCD",
            Self::SquareX => "Square-X",
            Self::TouchpadLock => "Touchpad lock",
            Self::Ai => "AI",
        }
    }

    /// COSMIC keysym produced by the native HID identity or firmware chord.
    pub const fn trigger(self) -> &'static str {
        match self {
            // The standard XKB inet(evdev) map translates Linux KEY_F13-F17
            // into these keysyms before COSMIC matches global shortcuts.
            Self::BrightnessDown => "XF86Tools",
            Self::BrightnessUp => "XF86Launch5",
            Self::Fan => "XF86Launch6",
            Self::Sleep => "XF86Launch7",
            Self::Wifi => "XF86Launch8",
            // These two buttons already emit unique native keyboard chords on
            // interface 0. Reuse them instead of adding a second HID-BPF
            // translation that would duplicate each physical press.
            Self::Display => "Super+p",
            Self::SquareX => "F19",
            Self::TouchpadLock => "Super+Ctrl+F24",
            // F21 is intentionally unused: the firmware's airplane-mode
            // report already has a native Linux input path.
            Self::Ai => "F22",
        }
    }

    /// Trigger written by development builds before the native interface-0
    /// reports were wired directly. It is accepted only so the next save can
    /// replace the exact AORUS-owned entry.
    pub const fn legacy_trigger(self) -> Option<&'static str> {
        match self {
            Self::BrightnessDown => Some("F13"),
            Self::BrightnessUp => Some("F14"),
            Self::Fan => Some("F15"),
            Self::Sleep => Some("F16"),
            Self::Wifi => Some("F17"),
            Self::Display => Some("F18"),
            Self::TouchpadLock => Some("F20"),
            _ => None,
        }
    }

    pub const fn default_action(self) -> FnAction {
        match self {
            Self::BrightnessDown => FnAction::BrightnessDown,
            Self::BrightnessUp => FnAction::BrightnessUp,
            Self::Fan => FnAction::CyclePowerProfile,
            Self::Sleep => FnAction::Suspend,
            Self::Wifi => FnAction::WifiToggle,
            Self::Display => FnAction::DisplayToggle,
            Self::SquareX | Self::Ai => FnAction::OpenApp,
            Self::TouchpadLock => FnAction::TouchpadToggle,
        }
    }

    pub const fn evidence(self) -> ButtonEvidence {
        match self {
            Self::BrightnessDown => ButtonEvidence::Captured("04 00 00 7d"),
            Self::BrightnessUp => ButtonEvidence::Captured("04 00 00 7e"),
            Self::Fan => ButtonEvidence::Captured("04 00 00 84"),
            Self::Sleep => ButtonEvidence::Captured("02 02 press; 02 00 release"),
            Self::Wifi => ButtonEvidence::Captured("04 00 00 7c"),
            Self::Display => ButtonEvidence::Captured("interface-0 Super+P press/release sequence"),
            Self::SquareX => ButtonEvidence::Captured("04 00 00 80"),
            Self::TouchpadLock => {
                ButtonEvidence::Captured("04 00 00 81 plus interface-0 keyboard sequence")
            }
            Self::Ai => ButtonEvidence::Captured("04 00 00 88"),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|button| button.id() == id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonEvidence {
    Captured(&'static str),
    NotCaptured,
}

impl ButtonEvidence {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Captured(_) => "Captured native trigger",
            Self::NotCaptured => "Not captured",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FnAction {
    Disabled,
    BrightnessDown,
    BrightnessUp,
    PowerBattery,
    PowerBalanced,
    PowerPerformance,
    CyclePowerProfile,
    FanNormal,
    FanSilent,
    FanGaming,
    FanCustom,
    FanReapply,
    Suspend,
    WifiToggle,
    DisplayToggle,
    TouchpadToggle,
    AirplaneToggle,
    Screenshot,
    VolumeDown,
    VolumeUp,
    VolumeMute,
    MediaPlayPause,
    OpenApp,
}

impl FnAction {
    pub const ALL: [Self; 23] = [
        Self::Disabled,
        Self::BrightnessDown,
        Self::BrightnessUp,
        Self::PowerBattery,
        Self::PowerBalanced,
        Self::PowerPerformance,
        Self::CyclePowerProfile,
        Self::FanNormal,
        Self::FanSilent,
        Self::FanGaming,
        Self::FanCustom,
        Self::FanReapply,
        Self::Suspend,
        Self::WifiToggle,
        Self::DisplayToggle,
        Self::TouchpadToggle,
        Self::AirplaneToggle,
        Self::Screenshot,
        Self::VolumeDown,
        Self::VolumeUp,
        Self::VolumeMute,
        Self::MediaPlayPause,
        Self::OpenApp,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::BrightnessDown => "brightness-down",
            Self::BrightnessUp => "brightness-up",
            Self::PowerBattery => "power-battery",
            Self::PowerBalanced => "power-balanced",
            Self::PowerPerformance => "power-performance",
            Self::CyclePowerProfile => "cycle-power-profile",
            Self::FanNormal => "fan-normal",
            Self::FanSilent => "fan-silent",
            Self::FanGaming => "fan-gaming",
            Self::FanCustom => "fan-custom",
            Self::FanReapply => "fan-reapply",
            Self::Suspend => "suspend",
            Self::WifiToggle => "wifi-toggle",
            Self::DisplayToggle => "display-toggle",
            Self::TouchpadToggle => "touchpad-toggle",
            Self::AirplaneToggle => "airplane-toggle",
            Self::Screenshot => "screenshot",
            Self::VolumeDown => "volume-down",
            Self::VolumeUp => "volume-up",
            Self::VolumeMute => "volume-mute",
            Self::MediaPlayPause => "media-play-pause",
            Self::OpenApp => "open-app",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::BrightnessDown => "Brightness down",
            Self::BrightnessUp => "Brightness up",
            Self::PowerBattery => "Power: Battery",
            Self::PowerBalanced => "Power: Balanced",
            Self::PowerPerformance => "Power: Performance",
            Self::CyclePowerProfile => "Power: Cycle profile",
            Self::FanNormal => "Fan: Normal profile",
            Self::FanSilent => "Fan: Silent profile",
            Self::FanGaming => "Fan: Gaming profile",
            Self::FanCustom => "Fan: Custom curve",
            Self::FanReapply => "Fan: Reapply mapped profile",
            Self::Suspend => "System: Suspend",
            Self::WifiToggle => "System: Toggle Wi-Fi",
            Self::DisplayToggle => "System: Toggle display mode",
            Self::TouchpadToggle => "System: Toggle touchpad",
            Self::AirplaneToggle => "System: Toggle airplane mode",
            Self::Screenshot => "System: Screenshot",
            Self::VolumeDown => "Media: Volume down",
            Self::VolumeUp => "Media: Volume up",
            Self::VolumeMute => "Media: Mute volume",
            Self::MediaPlayPause => "Media: Play / pause",
            Self::OpenApp => "Application: Open AORUS Control",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FnButtonMappings(BTreeMap<PhysicalButtonId, FnAction>);

impl Default for FnButtonMappings {
    fn default() -> Self {
        Self(
            PhysicalButtonId::ALL
                .into_iter()
                .map(|button| (button, button.default_action()))
                .collect(),
        )
    }
}

impl FnButtonMappings {
    pub fn get(&self, button: PhysicalButtonId) -> FnAction {
        self.0
            .get(&button)
            .copied()
            .unwrap_or_else(|| button.default_action())
    }

    pub fn set(&mut self, button: PhysicalButtonId, action: FnAction) {
        self.0.insert(button, action);
    }

    pub fn reset(&mut self, button: PhysicalButtonId) {
        self.set(button, button.default_action());
    }

    pub fn reset_all(&mut self) {
        *self = Self::default();
    }

    pub fn iter(&self) -> impl Iterator<Item = (PhysicalButtonId, FnAction)> + '_ {
        PhysicalButtonId::ALL
            .into_iter()
            .map(|button| (button, self.get(button)))
    }
}

#[derive(Debug)]
pub enum FnButtonError {
    ConfigHome,
    Io(&'static str, PathBuf, io::Error),
    Parse(String),
    UnsupportedVersion(u32),
    UnknownButton(String),
    UnknownAction(String),
    Readback,
}

impl fmt::Display for FnButtonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigHome => f.write_str("cannot locate the user configuration directory"),
            Self::Io(op, path, error) => write!(f, "{op} {}: {error}", path.display()),
            Self::Parse(error) => write!(f, "invalid Fn-button configuration: {error}"),
            Self::UnsupportedVersion(version) => write!(
                f,
                "Fn-button configuration version {version} is unsupported; no changes were made"
            ),
            Self::UnknownButton(button) => write!(f, "unknown physical button '{button}'"),
            Self::UnknownAction(action) => write!(f, "unknown Fn action '{action}'"),
            Self::Readback => f.write_str("Fn-button configuration readback did not match"),
        }
    }
}

impl std::error::Error for FnButtonError {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredMappings {
    version: u32,
    #[serde(default)]
    buttons: BTreeMap<String, String>,
}

pub fn load_mappings() -> Result<FnButtonMappings, FnButtonError> {
    load_mappings_from(config_path()?)
}

pub fn load_mappings_from(path: impl AsRef<Path>) -> Result<FnButtonMappings, FnButtonError> {
    let path = path.as_ref();
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(FnButtonMappings::default());
        }
        Err(error) => return Err(FnButtonError::Io("read", path.to_owned(), error)),
    };
    let stored: StoredMappings =
        toml::from_str(&text).map_err(|error| FnButtonError::Parse(error.to_string()))?;
    if stored.version != CONFIG_VERSION {
        return Err(FnButtonError::UnsupportedVersion(stored.version));
    }
    let mut mappings = FnButtonMappings::default();
    for (button, action) in stored.buttons {
        // Version-1 builds briefly exposed the already-native airplane key.
        // Drop that obsolete choice rather than making an upgrade fail.
        if button == "airplane-mode" {
            continue;
        }
        let button = PhysicalButtonId::from_id(&button)
            .ok_or_else(|| FnButtonError::UnknownButton(button.clone()))?;
        let action = FnAction::from_id(&action)
            .ok_or_else(|| FnButtonError::UnknownAction(action.clone()))?;
        mappings.set(button, action);
    }
    Ok(mappings)
}

pub fn save_mappings(mappings: &FnButtonMappings) -> Result<(), FnButtonError> {
    save_mappings_to(config_path()?, mappings)
}

pub fn save_mappings_to(
    path: impl AsRef<Path>,
    mappings: &FnButtonMappings,
) -> Result<(), FnButtonError> {
    let path = path.as_ref();
    let stored = StoredMappings {
        version: CONFIG_VERSION,
        buttons: mappings
            .iter()
            .filter(|(button, action)| *action != button.default_action())
            .map(|(button, action)| (button.id().to_owned(), action.id().to_owned()))
            .collect(),
    };
    let text =
        toml::to_string_pretty(&stored).map_err(|error| FnButtonError::Parse(error.to_string()))?;
    write_atomic(path, &text)?;
    if load_mappings_from(path)? != *mappings {
        return Err(FnButtonError::Readback);
    }
    Ok(())
}

fn config_path() -> Result<PathBuf, FnButtonError> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .ok_or(FnButtonError::ConfigHome)?;
    Ok(base.join("aorus-control/fn-buttons.toml"))
}

fn write_atomic(path: &Path, text: &str) -> Result<(), FnButtonError> {
    let parent = path.parent().ok_or(FnButtonError::ConfigHome)?;
    fs::create_dir_all(parent)
        .map_err(|error| FnButtonError::Io("create directory", parent.to_owned(), error))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".fn-buttons.toml.{}.{}.tmp",
        std::process::id(),
        stamp
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| FnButtonError::Io("create", temporary.clone(), error))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| FnButtonError::Io("write", temporary.clone(), error))?;
        fs::rename(&temporary, path)
            .map_err(|error| FnButtonError::Io("replace", path.to_owned(), error))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| FnButtonError::Io("set permissions", path.to_owned(), error))?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| FnButtonError::Io("sync directory", parent.to_owned(), error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "aorus-control-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn defaults_cover_every_button_and_triggers_are_unique() {
        let defaults = FnButtonMappings::default();
        let triggers: std::collections::HashSet<_> = PhysicalButtonId::ALL
            .into_iter()
            .map(PhysicalButtonId::trigger)
            .collect();
        assert_eq!(defaults.iter().count(), PhysicalButtonId::ALL.len());
        assert_eq!(triggers.len(), PhysicalButtonId::ALL.len());
        assert_eq!(PhysicalButtonId::Display.trigger(), "Super+p");
        assert_eq!(PhysicalButtonId::TouchpadLock.trigger(), "Super+Ctrl+F24");
    }

    #[test]
    fn round_trip_uses_defaults_for_missing_entries() {
        let path = temp_file("fn-roundtrip");
        let mut mappings = FnButtonMappings::default();
        mappings.set(PhysicalButtonId::Ai, FnAction::Screenshot);
        save_mappings_to(&path, &mappings).unwrap();
        assert_eq!(load_mappings_from(&path).unwrap(), mappings);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn unsupported_versions_fail_closed() {
        let path = temp_file("fn-version");
        fs::write(&path, "version = 99\n").unwrap();
        assert!(matches!(
            load_mappings_from(&path),
            Err(FnButtonError::UnsupportedVersion(99))
        ));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_airplane_choice_is_dropped_because_linux_owns_the_key() {
        let path = temp_file("fn-retired-airplane");
        fs::write(
            &path,
            "version = 1\n\n[buttons]\nairplane-mode = \"screenshot\"\nai = \"screenshot\"\n",
        )
        .unwrap();
        let mappings = load_mappings_from(&path).unwrap();
        assert_eq!(mappings.get(PhysicalButtonId::Ai), FnAction::Screenshot);
        assert!(PhysicalButtonId::from_id("airplane-mode").is_none());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn fan_actions_round_trip_for_write_mode() {
        let path = temp_file("fn-fan-action");
        fs::write(&path, "version = 1\n\n[buttons]\nfan = \"fan-gaming\"\n").unwrap();
        assert_eq!(
            load_mappings_from(&path)
                .unwrap()
                .get(PhysicalButtonId::Fan),
            FnAction::FanGaming
        );

        let mut mappings = FnButtonMappings::default();
        mappings.set(PhysicalButtonId::Fan, FnAction::FanGaming);
        let persisted = temp_file("fn-fan-action-save");
        save_mappings_to(&persisted, &mappings).unwrap();
        assert_eq!(load_mappings_from(&persisted).unwrap(), mappings);
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(persisted);
    }
}
