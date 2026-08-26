//! System76 power-profile access and normalization.

use std::time::Duration;

use zbus::blocking::{Connection, Proxy, connection::Builder};

use crate::model::PowerProfile;

const SYSTEM76_DESTINATION: &str = "com.system76.PowerDaemon";
const SYSTEM76_PATH: &str = "/com/system76/PowerDaemon";
const SYSTEM76_INTERFACE: &str = "com.system76.PowerDaemon";
const UPOWER_DESTINATION: &str = "org.freedesktop.UPower.PowerProfiles";
const UPOWER_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
const UPOWER_INTERFACE: &str = "org.freedesktop.UPower.PowerProfiles";
const METHOD_TIMEOUT: Duration = Duration::from_secs(10);

fn system_connection() -> Result<Connection, String> {
    Builder::system()
        .and_then(|builder| builder.method_timeout(METHOD_TIMEOUT).build())
        .map_err(|error| error.to_string())
}

pub fn normalize(value: &str) -> Option<PowerProfile> {
    value.trim().trim_matches('"').parse().ok()
}

fn system76_proxy(connection: &Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(
        connection,
        SYSTEM76_DESTINATION,
        SYSTEM76_PATH,
        SYSTEM76_INTERFACE,
    )
    .map_err(|error| error.to_string())
}

fn upower_proxy(connection: &Connection) -> Result<Proxy<'_>, String> {
    Proxy::new(
        connection,
        UPOWER_DESTINATION,
        UPOWER_PATH,
        UPOWER_INTERFACE,
    )
    .map_err(|error| error.to_string())
}

fn current_profile_on(connection: &Connection) -> Result<PowerProfile, String> {
    let system76_error = match system76_proxy(connection) {
        Ok(proxy) => match proxy.call::<_, _, String>("GetProfile", &()) {
            Ok(value) => {
                return normalize(&value).ok_or_else(|| {
                    format!("System76 returned unsupported power profile {value:?}")
                });
            }
            Err(error) => error.to_string(),
        },
        Err(error) => error,
    };

    let upower_error = match upower_proxy(connection) {
        Ok(proxy) => match proxy.get_property::<String>("ActiveProfile") {
            Ok(value) => {
                return normalize(&value)
                    .ok_or_else(|| format!("UPower returned unsupported power profile {value:?}"));
            }
            Err(error) => error.to_string(),
        },
        Err(error) => error,
    };

    Err(format!(
        "no power profile available (System76: {system76_error}; UPower: {upower_error})"
    ))
}

/// Read System76 first, then the standard UPower PowerProfiles property.
pub fn current_profile() -> Result<PowerProfile, String> {
    let connection = system_connection()?;
    current_profile_on(&connection)
}

/// Same as [`current_profile`], reusing a caller-owned system-bus connection.
pub fn current_profile_with(connection: &Connection) -> Result<PowerProfile, String> {
    current_profile_on(connection)
}

/// Set the CPU/system policy through System76's native profile methods.
pub fn set_profile(profile: PowerProfile) -> Result<(), String> {
    let connection = system_connection()?;
    system76_proxy(&connection)?
        .call::<_, _, ()>(system76_method(profile), &())
        .map_err(|error| format!("System76 {} failed: {error}", system76_method(profile)))?;
    let actual = current_profile_with(&connection)?;
    if actual == profile {
        Ok(())
    } else {
        Err(format!(
            "System76 reported {actual} after requesting {profile}; fan mapping was not changed"
        ))
    }
}

pub fn system_state() -> (Option<PowerProfile>, Option<String>, Option<bool>) {
    let Ok(connection) = system_connection() else {
        return (None, None, None);
    };
    let profile = current_profile_with(&connection).ok();
    let Ok(proxy) = system76_proxy(&connection) else {
        return (profile, None, None);
    };
    let graphics_mode = proxy.call::<_, _, String>("GetGraphics", &()).ok();
    let graphics_power = proxy.call::<_, _, bool>("GetGraphicsPower", &()).ok();
    (profile, graphics_mode, graphics_power)
}

fn system76_method(profile: PowerProfile) -> &'static str {
    match profile {
        PowerProfile::Performance => "Performance",
        PowerProfile::Balanced => "Balanced",
        PowerProfile::Battery => "Battery",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_system76_and_upower_spellings() {
        assert_eq!(normalize("Performance"), Some(PowerProfile::Performance));
        assert_eq!(normalize("balanced"), Some(PowerProfile::Balanced));
        assert_eq!(normalize("Battery"), Some(PowerProfile::Battery));
        assert_eq!(normalize("power-saver"), Some(PowerProfile::Battery));
        assert_eq!(normalize("not-a-profile"), None);
    }
}
