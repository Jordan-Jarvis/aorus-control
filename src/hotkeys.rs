//! User-owned COSMIC global shortcut configuration.

use std::{
    collections::HashSet,
    env, fmt, fs,
    hash::{Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use ron::value::RawValue;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{MapAccess, Visitor},
    ser::SerializeMap,
};

pub const COSMIC_SHORTCUTS_ID: &str = "com.system76.CosmicSettings.Shortcuts";
const OWNER: &str = "AORUS Control: ";
const FN_OWNER: &str = "AORUS Control: fn-button:";
const DEFAULTS: &str = "/usr/share/cosmic/com.system76.CosmicSettings.Shortcuts/v1/defaults";
const INSTALL_PREFIX: &str = env!("AORUS_CONTROL_PREFIX");

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HotkeyAction {
    OpenApp,
    PowerBattery,
    PowerBalanced,
    PowerPerformance,
    FanNormal,
    FanSilent,
    FanGaming,
    FanCustom,
    FanReapply,
    BrightnessDown,
    BrightnessUp,
    VolumeDown,
    VolumeUp,
    VolumeMute,
    MediaPlayPause,
}

impl HotkeyAction {
    pub const ALL: [Self; 15] = [
        Self::OpenApp,
        Self::PowerBattery,
        Self::PowerBalanced,
        Self::PowerPerformance,
        Self::FanNormal,
        Self::FanSilent,
        Self::FanGaming,
        Self::FanCustom,
        Self::FanReapply,
        Self::BrightnessDown,
        Self::BrightnessUp,
        Self::VolumeDown,
        Self::VolumeUp,
        Self::VolumeMute,
        Self::MediaPlayPause,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenApp => "Open AORUS Control",
            Self::PowerBattery => "Power: Battery",
            Self::PowerBalanced => "Power: Balanced",
            Self::PowerPerformance => "Power: Performance",
            Self::FanNormal => "Fan: Normal",
            Self::FanSilent => "Fan: Silent",
            Self::FanGaming => "Fan: Gaming",
            Self::FanCustom => "Fan: Custom",
            Self::FanReapply => "Fan: Reapply",
            Self::BrightnessDown => "Brightness down",
            Self::BrightnessUp => "Brightness up",
            Self::VolumeDown => "Volume down",
            Self::VolumeUp => "Volume up",
            Self::VolumeMute => "Mute volume",
            Self::MediaPlayPause => "Play / pause media",
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::OpenApp => "open-app",
            Self::PowerBattery => "power-battery",
            Self::PowerBalanced => "power-balanced",
            Self::PowerPerformance => "power-performance",
            Self::FanNormal => "fan-normal",
            Self::FanSilent => "fan-silent",
            Self::FanGaming => "fan-gaming",
            Self::FanCustom => "fan-custom",
            Self::FanReapply => "fan-reapply",
            Self::BrightnessDown => "brightness-down",
            Self::BrightnessUp => "brightness-up",
            Self::VolumeDown => "volume-down",
            Self::VolumeUp => "volume-up",
            Self::VolumeMute => "volume-mute",
            Self::MediaPlayPause => "media-play-pause",
        }
    }

    fn command(self) -> Option<String> {
        let command = match self {
            Self::OpenApp => "aorus-control".to_owned(),
            Self::PowerBattery => "aorusctl profile battery".to_owned(),
            Self::PowerBalanced => "aorusctl profile balanced".to_owned(),
            Self::PowerPerformance => "aorusctl profile performance".to_owned(),
            Self::FanNormal => "aorusctl fan normal".to_owned(),
            Self::FanSilent => "aorusctl fan silent".to_owned(),
            Self::FanGaming => "aorusctl fan gaming".to_owned(),
            Self::FanCustom => "aorusctl fan custom".to_owned(),
            Self::FanReapply => "aorusctl fan reapply".to_owned(),
            _ => return None,
        };
        Some(format!("{INSTALL_PREFIX}/bin/{command}"))
    }

    fn ron(self) -> String {
        match self {
            Self::BrightnessDown => "System(BrightnessDown)".to_owned(),
            Self::BrightnessUp => "System(BrightnessUp)".to_owned(),
            Self::VolumeDown => "System(VolumeLower)".to_owned(),
            Self::VolumeUp => "System(VolumeRaise)".to_owned(),
            Self::VolumeMute => "System(Mute)".to_owned(),
            Self::MediaPlayPause => "System(PlayPause)".to_owned(),
            _ => format!("Spawn(\"{}\")", self.command().expect("spawn action")),
        }
    }

    fn configured_action(self) -> ConfiguredAction {
        match self {
            Self::BrightnessDown => ConfiguredAction::System(SystemAction::BrightnessDown),
            Self::BrightnessUp => ConfiguredAction::System(SystemAction::BrightnessUp),
            Self::VolumeDown => ConfiguredAction::System(SystemAction::VolumeLower),
            Self::VolumeUp => ConfiguredAction::System(SystemAction::VolumeRaise),
            Self::VolumeMute => ConfiguredAction::System(SystemAction::Mute),
            Self::MediaPlayPause => ConfiguredAction::System(SystemAction::PlayPause),
            _ => ConfiguredAction::Spawn(self.command().expect("spawn action")),
        }
    }

    fn from_description(value: &str) -> Option<Self> {
        let id = value.strip_prefix(OWNER)?;
        Self::ALL.into_iter().find(|action| action.id() == id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HotkeyMapping {
    pub action: HotkeyAction,
    pub binding: Option<String>,
}

#[derive(Debug)]
pub enum HotkeyError {
    Io(&'static str, PathBuf, io::Error),
    ConfigHome,
    UnsupportedDesktop,
    Parse(String),
    Invalid(String),
    Conflict(String),
    Duplicate(HotkeyAction),
    Modified(HotkeyAction),
    OwnedShortcutModified(String),
    Readback,
    Rollback(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(op, path, error) => write!(f, "{op} {}: {error}", path.display()),
            Self::ConfigHome => f.write_str("cannot locate the user configuration directory"),
            Self::UnsupportedDesktop => f.write_str("global shortcut editing requires COSMIC"),
            Self::Parse(error) => write!(f, "invalid COSMIC shortcut configuration: {error}"),
            Self::Invalid(error) => write!(f, "invalid hotkey: {error}"),
            Self::Conflict(binding) => write!(f, "{binding} is already assigned"),
            Self::Duplicate(action) => write!(f, "{} is listed twice", action.label()),
            Self::Modified(action) => write!(
                f,
                "the saved action for {} was modified outside AORUS Control",
                action.label()
            ),
            Self::OwnedShortcutModified(description) => write!(
                f,
                "the saved AORUS-owned shortcut {description:?} was modified outside AORUS Control"
            ),
            Self::Readback => {
                f.write_str("COSMIC shortcut readback did not match the saved values")
            }
            Self::Rollback(error) => write!(f, "shortcut save failed and rollback failed: {error}"),
        }
    }
}

impl std::error::Error for HotkeyError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
enum ConfiguredAction {
    Spawn(String),
    System(SystemAction),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
enum SystemAction {
    BrightnessDown,
    BrightnessUp,
    VolumeLower,
    VolumeRaise,
    Mute,
    PlayPause,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
enum Modifier {
    Super,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Clone, Debug, Deserialize, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    #[serde(default)]
    modifiers: Vec<Modifier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keycode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

// COSMIC deliberately ignores descriptions and keycodes when comparing bindings.
impl PartialEq for Binding {
    fn eq(&self, other: &Self) -> bool {
        modifier_bits(&self.modifiers) == modifier_bits(&other.modifiers) && self.key == other.key
    }
}
impl Hash for Binding {
    fn hash<H: Hasher>(&self, state: &mut H) {
        modifier_bits(&self.modifiers).hash(state);
        self.key.hash(state);
    }
}

fn modifier_bits(modifiers: &[Modifier]) -> u8 {
    modifiers.iter().fold(0, |bits, modifier| {
        bits | match modifier {
            Modifier::Super => 1,
            Modifier::Ctrl => 2,
            Modifier::Alt => 4,
            Modifier::Shift => 8,
        }
    })
}

#[derive(Default)]
struct Shortcuts(Vec<(Binding, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Shortcuts {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Shortcuts;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a COSMIC shortcut map")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(Shortcuts(entries))
            }
        }
        deserializer.deserialize_map(V)
    }
}
impl Serialize for Shortcuts {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (binding, action) in &self.0 {
            map.serialize_entry(binding, action)?;
        }
        map.end()
    }
}

pub fn backend_available() -> Result<(), String> {
    let desktop = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    desktop_supported(&desktop)
        .then_some(())
        .ok_or_else(|| HotkeyError::UnsupportedDesktop.to_string())
}

pub fn load_mappings() -> Result<Vec<HotkeyMapping>, String> {
    backend_available()?;
    load_mappings_from(config_path().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

pub fn load_mappings_from(path: impl AsRef<Path>) -> Result<Vec<HotkeyMapping>, HotkeyError> {
    mappings_from(&read(path.as_ref())?)
}

fn mappings_from(shortcuts: &Shortcuts) -> Result<Vec<HotkeyMapping>, HotkeyError> {
    let mut mappings = Vec::new();
    for (binding, value) in &shortcuts.0 {
        let Some(description) = binding.description.as_deref() else {
            continue;
        };
        // Ignore physical Fn-button entries left by older COSMIC-only builds.
        // They are removed separately after the native HID translation loads.
        if description.starts_with(FN_OWNER) {
            continue;
        }
        let Some(owner_action) = description.strip_prefix(OWNER) else {
            continue;
        };
        let action = HotkeyAction::from_description(description)
            .ok_or_else(|| HotkeyError::OwnedShortcutModified(owner_action.to_owned()))?;
        if !configured_action_matches(value.into_rust::<ConfiguredAction>(), action) {
            return Err(HotkeyError::Modified(action));
        }
        mappings.push(HotkeyMapping {
            action,
            binding: Some(binding_string(binding)?),
        });
    }
    normalize_mappings(&mappings)
}

fn configured_action_matches(
    configured: Result<ConfiguredAction, ron::de::SpannedError>,
    action: HotkeyAction,
) -> bool {
    let Ok(configured) = configured else {
        return false;
    };
    let expected = action.configured_action();
    if configured == expected {
        return true;
    }
    // Preserve shortcuts written by the source installer when upgrading to
    // the Debian package, whose binaries live under /usr/bin.
    match (configured, expected) {
        (ConfiguredAction::Spawn(actual), ConfiguredAction::Spawn(expected)) => actual
            .strip_prefix("/usr/local/bin/")
            .zip(expected.strip_prefix("/usr/bin/"))
            .is_some_and(|(actual, expected)| actual == expected),
        _ => false,
    }
}

pub fn save_mappings(mappings: &[HotkeyMapping]) -> Result<(), String> {
    backend_available()?;
    save_mappings_to(
        config_path().map_err(|error| error.to_string())?,
        DEFAULTS,
        mappings,
    )
    .map_err(|error| error.to_string())
}

pub fn save_mappings_to(
    path: impl AsRef<Path>,
    defaults: impl AsRef<Path>,
    mappings: &[HotkeyMapping],
) -> Result<(), HotkeyError> {
    let path = path.as_ref();
    let normalized = normalize_mappings(mappings)?;
    let mut custom = read(path)?;
    let defaults = read(defaults.as_ref())?;
    // Refuse to overwrite an app-owned entry that somebody changed behind our
    // back. Otherwise a save could silently discard that change.
    mappings_from(&custom)?;
    custom.0.retain(|(binding, _)| {
        binding
            .description
            .as_deref()
            .is_none_or(|value| HotkeyAction::from_description(value).is_none())
    });

    for mapping in &normalized {
        let Some(text) = &mapping.binding else {
            continue;
        };
        let binding = parse_binding(text, Some(mapping.action))?;
        if custom
            .0
            .iter()
            .chain(&defaults.0)
            .any(|(existing, _)| existing == &binding)
        {
            return Err(HotkeyError::Conflict(text.clone()));
        }
        let action = RawValue::from_boxed_ron(mapping.action.ron().into())
            .map_err(|error| HotkeyError::Parse(error.to_string()))?;
        custom.0.push((binding, action));
    }

    let output = serialize_shortcuts(&custom)?;
    let original = match fs::read(path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(HotkeyError::Io("read", path.to_owned(), error)),
    };
    if let Err(error) = write_atomic(path, &(output + "\n")) {
        return Err(rollback_file(path, original.as_deref(), error));
    }
    if let Err(error) = (|| {
        if mappings_from(&read(path)?)? != normalized {
            return Err(HotkeyError::Readback);
        }
        Ok(())
    })() {
        return Err(rollback_file(path, original.as_deref(), error));
    }
    Ok(())
}

/// Remove physical Fn-button records created by older COSMIC-only releases.
/// Physical mappings are now system-wide native input mappings; conventional
/// user shortcuts remain untouched.
pub fn remove_legacy_fn_button_shortcuts() -> Result<(), String> {
    if backend_available().is_err() {
        return Ok(());
    }
    let path = config_path().map_err(|error| error.to_string())?;
    let mut shortcuts = read(&path).map_err(|error| error.to_string())?;
    let before = shortcuts.0.len();
    shortcuts.0.retain(|(binding, _)| {
        !binding
            .description
            .as_deref()
            .is_some_and(|description| description.starts_with(FN_OWNER))
    });
    if shortcuts.0.len() == before {
        return Ok(());
    }
    let text = serialize_shortcuts(&shortcuts).map_err(|error| error.to_string())? + "\n";
    write_atomic(&path, &text).map_err(|error| error.to_string())
}

fn rollback_file(path: &Path, original: Option<&[u8]>, error: HotkeyError) -> HotkeyError {
    match restore_file(path, original) {
        Ok(()) => error,
        Err(rollback) => HotkeyError::Rollback(format!("{error}; {rollback}")),
    }
}

fn restore_file(path: &Path, contents: Option<&[u8]>) -> Result<(), HotkeyError> {
    match contents {
        Some(contents) => {
            let text = std::str::from_utf8(contents)
                .map_err(|error| HotkeyError::Parse(error.to_string()))?;
            write_atomic(path, text)
        }
        None => match fs::remove_file(path) {
            Ok(()) => sync_parent(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(HotkeyError::Io("remove", path.to_owned(), error)),
        },
    }
}

fn sync_parent(path: &Path) -> Result<(), HotkeyError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| HotkeyError::Io("sync directory", parent.to_owned(), error))
}

pub fn normalize_binding(input: &str) -> Result<String, HotkeyError> {
    let mut modifiers = [false; 4];
    let mut key = None;
    for token in input.split('+').map(str::trim) {
        if token.is_empty() {
            return Err(HotkeyError::Invalid("empty key".to_owned()));
        }
        let modifier = match token.to_ascii_lowercase().as_str() {
            "super" | "logo" | "meta" => Some(0),
            "ctrl" | "control" => Some(1),
            "alt" => Some(2),
            "shift" => Some(3),
            _ => None,
        };
        if let Some(index) = modifier {
            if std::mem::replace(&mut modifiers[index], true) {
                return Err(HotkeyError::Invalid(format!("duplicate modifier {token}")));
            }
        } else if key.replace(canonical_key(token)?).is_some() {
            return Err(HotkeyError::Invalid(
                "only one non-modifier key is allowed".to_owned(),
            ));
        }
    }
    let key = match key {
        Some(key) => key,
        None if modifiers == [true, false, false, false] => return Ok("Super".to_owned()),
        None => return Err(HotkeyError::Invalid("missing key".to_owned())),
    };
    if !modifiers.iter().any(|value| *value) && is_typing_key(&key) {
        return Err(HotkeyError::Invalid(
            "ordinary typing keys require a modifier".to_owned(),
        ));
    }
    let mut parts: Vec<&str> = modifiers
        .into_iter()
        .zip(["Super", "Ctrl", "Alt", "Shift"])
        .filter_map(|(set, name)| set.then_some(name))
        .collect();
    parts.push(&key);
    Ok(parts.join("+"))
}

fn normalize_mappings(mappings: &[HotkeyMapping]) -> Result<Vec<HotkeyMapping>, HotkeyError> {
    let mut actions = HashSet::new();
    let mut bindings = HashSet::new();
    let mut result = Vec::with_capacity(mappings.len());
    for mapping in mappings {
        if !actions.insert(mapping.action) {
            return Err(HotkeyError::Duplicate(mapping.action));
        }
        let binding = mapping
            .binding
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(normalize_binding)
            .transpose()?;
        if binding
            .as_ref()
            .is_some_and(|value| !bindings.insert(value.clone()))
        {
            return Err(HotkeyError::Conflict(binding.unwrap()));
        }
        result.push(HotkeyMapping {
            action: mapping.action,
            binding,
        });
    }
    result.sort_by_key(|mapping| {
        HotkeyAction::ALL
            .iter()
            .position(|action| action == &mapping.action)
            .unwrap()
    });
    Ok(result)
}

fn parse_binding(text: &str, action: Option<HotkeyAction>) -> Result<Binding, HotkeyError> {
    let text = normalize_binding(text)?;
    let mut tokens: Vec<_> = text.split('+').collect();
    let key = tokens.pop().map(str::to_owned);
    let modifiers = tokens
        .into_iter()
        .map(|token| match token {
            "Super" => Modifier::Super,
            "Ctrl" => Modifier::Ctrl,
            "Alt" => Modifier::Alt,
            "Shift" => Modifier::Shift,
            _ => unreachable!(),
        })
        .collect();
    Ok(Binding {
        modifiers,
        key,
        keycode: None,
        description: action.map(|action| format!("{OWNER}{}", action.id())),
    })
}

fn binding_string(binding: &Binding) -> Result<String, HotkeyError> {
    let mut parts: Vec<&str> = binding
        .modifiers
        .iter()
        .map(|modifier| match modifier {
            Modifier::Super => "Super",
            Modifier::Ctrl => "Ctrl",
            Modifier::Alt => "Alt",
            Modifier::Shift => "Shift",
        })
        .collect();
    match binding.key.as_deref() {
        Some(key) => parts.push(key),
        None if modifier_bits(&binding.modifiers) == 1 => return Ok("Super".to_owned()),
        None => {
            return Err(HotkeyError::Invalid(
                "AORUS shortcuts require a key symbol".to_owned(),
            ));
        }
    }
    normalize_binding(&parts.join("+"))
}

fn desktop_supported(desktop: &str) -> bool {
    desktop
        .split([':', ';'])
        .map(str::trim)
        .any(|name| name.eq_ignore_ascii_case("cosmic"))
}

fn canonical_key(input: &str) -> Result<String, HotkeyError> {
    if input.chars().count() == 1 {
        let value = input.chars().next().unwrap();
        return value
            .is_ascii_alphanumeric()
            .then(|| value.to_ascii_lowercase().to_string())
            .ok_or_else(|| HotkeyError::Invalid(format!("unknown key {input}")));
    }
    let lower = input.to_ascii_lowercase();
    if let Some(number) = lower
        .strip_prefix('f')
        .and_then(|value| value.parse::<u8>().ok())
        && (1..=35).contains(&number)
    {
        return Ok(format!("F{number}"));
    }
    let value = match lower.as_str() {
        "esc" | "escape" => "Escape",
        "enter" | "return" => "Return",
        "tab" => "Tab",
        "space" => "space",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "home" => "Home",
        "end" => "End",
        "insert" => "Insert",
        "delete" => "Delete",
        "backspace" => "BackSpace",
        "pageup" => "Page_Up",
        "pagedown" => "Page_Down",
        "comma" => "comma",
        "period" => "period",
        "minus" => "minus",
        "equals" | "equal" => "equal",
        "slash" => "slash",
        "backslash" => "backslash",
        "semicolon" => "semicolon",
        "quote" | "apostrophe" => "apostrophe",
        "openbracket" | "bracketleft" => "bracketleft",
        "closebracket" | "bracketright" => "bracketright",
        "backtick" | "grave" => "grave",
        "print" => "Print",
        "xf86monbrightnessdown" => "XF86MonBrightnessDown",
        "xf86monbrightnessup" => "XF86MonBrightnessUp",
        "xf86audiolowervolume" => "XF86AudioLowerVolume",
        "xf86audioraisevolume" => "XF86AudioRaiseVolume",
        "xf86audiomute" => "XF86AudioMute",
        "xf86audioplay" => "XF86AudioPlay",
        "xf86audioprev" => "XF86AudioPrev",
        "xf86audionext" => "XF86AudioNext",
        "xf86audiomicmute" => "XF86AudioMicMute",
        "xf86poweroff" => "XF86PowerOff",
        "xf86touchtoggle" | "xf86touchpadtoggle" => "XF86TouchpadToggle",
        "xf86launcha" => "XF86LaunchA",
        "xf86tools" => "XF86Tools",
        "xf86launch5" => "XF86Launch5",
        "xf86launch6" => "XF86Launch6",
        "xf86launch7" => "XF86Launch7",
        "xf86launch8" => "XF86Launch8",
        _ => return Err(HotkeyError::Invalid(format!("unknown key {input}"))),
    };
    Ok(value.to_owned())
}

fn is_typing_key(key: &str) -> bool {
    key.chars().count() == 1
        || matches!(
            key,
            "space" | "Tab" | "Return" | "comma" | "period" | "minus" | "equal" | "slash"
        )
}
fn read(path: &Path) -> Result<Shortcuts, HotkeyError> {
    match fs::read_to_string(path) {
        Ok(text) => ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
            .from_str(&text)
            .map_err(|error| HotkeyError::Parse(error.to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Shortcuts::default()),
        Err(error) => Err(HotkeyError::Io("read", path.to_owned(), error)),
    }
}

fn serialize_shortcuts(shortcuts: &Shortcuts) -> Result<String, HotkeyError> {
    let pretty =
        ron::ser::PrettyConfig::default().extensions(ron::extensions::Extensions::IMPLICIT_SOME);
    ron::ser::to_string_pretty(shortcuts, pretty)
        .map_err(|error| HotkeyError::Parse(error.to_string()))
}

fn config_path() -> Result<PathBuf, HotkeyError> {
    let root = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .ok_or(HotkeyError::ConfigHome)?;
    Ok(root
        .join("cosmic")
        .join(COSMIC_SHORTCUTS_ID)
        .join("v1/custom"))
}

fn write_atomic(path: &Path, text: &str) -> Result<(), HotkeyError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| HotkeyError::Io("create directory", parent.to_owned(), error))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".custom.{stamp}.tmp"));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| HotkeyError::Io("create", temporary.clone(), error))?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|error| HotkeyError::Io("write", temporary.clone(), error))?;
        fs::rename(&temporary, path)
            .map_err(|error| HotkeyError::Io("replace", path.to_owned(), error))
            .and_then(|()| sync_parent(path))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (PathBuf, PathBuf, PathBuf) {
        let root = env::temp_dir().join(format!(
            "aorus-hotkeys-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        (root.join("custom"), root.join("defaults"), root)
    }
    fn mapping(action: HotkeyAction, binding: &str) -> HotkeyMapping {
        HotkeyMapping {
            action,
            binding: Some(binding.to_owned()),
        }
    }

    #[test]
    fn validates_bindings() {
        assert_eq!(
            normalize_binding("shift + super + f9").unwrap(),
            "Super+Shift+F9"
        );
        assert_eq!(normalize_binding("logo").unwrap(), "Super");
        assert!(normalize_binding("A").is_err());
        assert!(normalize_binding("Super+F9+F10").is_err());
        assert!(normalize_binding("XF86MonBrightnessDown").is_ok());
        assert_eq!(normalize_binding("print").unwrap(), "Print");
    }

    #[test]
    fn gates_only_cosmic_desktops() {
        assert!(desktop_supported("COSMIC"));
        assert!(desktop_supported("cosmic:GNOME"));
        assert!(desktop_supported("GNOME;COSMIC"));
        assert!(!desktop_supported("GNOME:KDE"));
        assert!(!desktop_supported(""));
    }

    #[test]
    fn preserves_other_shortcuts_and_replaces_owned_entries() {
        let (custom, defaults, root) = paths();
        fs::create_dir_all(&root).unwrap();
        fs::write(&custom, "{(modifiers:[Super],key:\"q\"):Close,(modifiers:[Super],key:\"F9\",description:Some(\"AORUS Control: open-app\")):Spawn(\"/usr/local/bin/aorus-control\")}").unwrap();
        fs::write(&defaults, "{}").unwrap();
        save_mappings_to(
            &custom,
            &defaults,
            &[mapping(HotkeyAction::PowerPerformance, "Super+F10")],
        )
        .unwrap();
        let text = fs::read_to_string(&custom).unwrap();
        assert!(text.contains("Close"));
        assert!(!text.contains("open-app"));
        assert!(!text.contains("Some("));
        assert_eq!(
            load_mappings_from(&custom).unwrap(),
            vec![mapping(HotkeyAction::PowerPerformance, "Super+F10")]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_default_conflicts_without_writing() {
        let (custom, defaults, root) = paths();
        fs::create_dir_all(&root).unwrap();
        fs::write(&custom, "{}").unwrap();
        fs::write(&defaults, "{(modifiers:[Super],key:\"F9\"):Close}").unwrap();
        assert!(
            save_mappings_to(
                &custom,
                &defaults,
                &[mapping(HotkeyAction::OpenApp, "Super+F9")]
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&custom).unwrap(), "{}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_existing_owned_changes_without_writing() {
        let (custom, defaults, root) = paths();
        fs::create_dir_all(&root).unwrap();
        let original =
            "{(modifiers:[Super],key:\"F9\",description:Some(\"AORUS Control: open-app\")):Close}";
        fs::write(&custom, original).unwrap();
        fs::write(&defaults, "{}").unwrap();

        assert!(matches!(
            save_mappings_to(
                &custom,
                &defaults,
                &[mapping(HotkeyAction::OpenApp, "Super+F10")]
            ),
            Err(HotkeyError::Modified(HotkeyAction::OpenApp))
        ));
        assert_eq!(fs::read_to_string(&custom).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_conventional_owned_entries_without_writing() {
        let (custom, defaults, root) = paths();
        fs::create_dir_all(&root).unwrap();
        let original =
            "{(modifiers:[Super],key:\"F9\",description:Some(\"AORUS Control: unknown\")):Close}";
        fs::write(&custom, original).unwrap();
        fs::write(&defaults, "{}").unwrap();

        assert!(matches!(
            save_mappings_to(
                &custom,
                &defaults,
                &[mapping(HotkeyAction::OpenApp, "Super+F10")]
            ),
            Err(HotkeyError::OwnedShortcutModified(value)) if value == "unknown"
        ));
        assert_eq!(fs::read_to_string(&custom).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_actions_semantically_and_rejects_duplicate_owned_entries() {
        let (custom, _defaults, root) = paths();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            &custom,
            "{(modifiers:[Super],key:\"F9\",description:Some(\"AORUS Control: brightness-up\")):System( BrightnessUp )}",
        )
        .unwrap();
        assert_eq!(
            load_mappings_from(&custom).unwrap(),
            vec![mapping(HotkeyAction::BrightnessUp, "Super+F9")]
        );

        fs::write(
            &custom,
            "{(modifiers:[Super],key:\"F9\",description:Some(\"AORUS Control: brightness-up\")):System(BrightnessUp),(modifiers:[Super],key:\"F10\",description:Some(\"AORUS Control: brightness-up\")):System(BrightnessUp)}",
        )
        .unwrap();
        assert!(matches!(
            load_mappings_from(&custom),
            Err(HotkeyError::Duplicate(HotkeyAction::BrightnessUp))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_custom_conflicts_without_writing() {
        let (custom, defaults, root) = paths();
        fs::create_dir_all(&root).unwrap();
        let original = "{(modifiers:[Ctrl],key:\"F9\"):Close}";
        fs::write(&custom, original).unwrap();
        fs::write(&defaults, "{}").unwrap();
        assert!(matches!(
            save_mappings_to(&custom, &defaults, &[mapping(HotkeyAction::OpenApp, "Ctrl+F9")]),
            Err(HotkeyError::Conflict(binding)) if binding == "Ctrl+F9"
        ));
        assert_eq!(fs::read_to_string(&custom).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_installed_cosmic_defaults_when_available() {
        let path = Path::new(DEFAULTS);
        if path.exists() {
            assert!(!read(path).unwrap().0.is_empty());
        }
    }
}
