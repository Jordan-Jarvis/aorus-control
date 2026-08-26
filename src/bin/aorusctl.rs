//! Small command-line client for the documented AORUS Control D-Bus API.
//!
//! All reads and writes go through aorusd, so the CLI and native UI share the
//! daemon's validation, authorization, and rollback behavior.

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::error::Error;
use std::fmt;
use std::process;
use std::time::Duration;

use aorus_control::curve::{FAN_CURVE_POINTS, FanCurve, FanPoint};
use aorus_control::{DBUS_DESTINATION, DBUS_INTERFACE, DBUS_PATH};
use zbus::blocking::{Connection, Proxy, connection::Builder};
use zbus::zvariant::OwnedValue;

const EXIT_USAGE: i32 = 2;
const EXIT_UNAVAILABLE: i32 = 3;
const EXIT_AUTH: i32 = 4;
const EXIT_VALIDATION: i32 = 5;
const EXIT_OPERATION: i32 = 6;
const DBUS_METHOD_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct CliError {
    code: i32,
    message: String,
}

impl CliError {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for CliError {}

type CliResult<T> = Result<T, CliError>;

fn usage() -> &'static str {
    "Usage:\n  aorusctl status\n  aorusctl curve show\n  aorusctl curve apply TEMP:RAW_SPEED ... (exactly 15 points)\n  aorusctl profile performance|balanced|battery\n  aorusctl fan normal|silent|gaming|custom\n  aorusctl fan reapply\n  aorusctl mappings get\n  aorusctl mappings set PROFILE=FAN_PROFILE [...]\n  aorusctl charge mode 0|1|normal|custom\n  aorusctl charge limit 60-100\n  aorusctl gpu boost VALUE\n  aorusctl diagnostics\n\nCurve points use temperature in °C and firmware raw fan level 0-255, for\nexample: aorusctl curve apply T0:S0 T1:S1 ... (15 total). Values are\nvalidated locally and again by aorusd; the CLI never writes sysfs.\n\nExit codes:\n  2  invalid command or argument\n  3  system bus or aorusd unavailable\n  4  authorization denied\n  5  invalid value rejected by aorusd\n  6  hardware operation, verification, or API failure"
}

fn classify_dbus_error(message: &str) -> i32 {
    let lower = message.to_ascii_lowercase();
    if lower.contains("accessdenied")
        || lower.contains("permission denied")
        || lower.contains("not authorized")
        || lower.contains("notauthorized")
        || lower.contains("authentication")
    {
        EXIT_AUTH
    } else if lower.contains("name has no owner")
        || lower.contains("serviceunknown")
        || lower.contains("no such service")
        || lower.contains("failed to connect to")
        || lower.contains("connection refused")
        || lower.contains("timeout")
        || lower.contains("timedout")
        || lower.contains("timed out")
        || lower.contains("cannot autolaunch")
    {
        EXIT_UNAVAILABLE
    } else if lower.contains("invalid")
        || lower.contains("validation")
        || lower.contains("out of range")
        || lower.contains("unsupported")
    {
        EXIT_VALIDATION
    } else {
        EXIT_OPERATION
    }
}

fn dbus_error(operation: &str, error: impl fmt::Display) -> CliError {
    let detail = error.to_string();
    CliError::new(
        classify_dbus_error(&detail),
        format!("{operation} failed: {detail}"),
    )
}

fn system_connection() -> CliResult<Connection> {
    let builder = Builder::system().map_err(|error| {
        dbus_error(
            "connecting to the system D-Bus",
            format_args!("{error}; is the system bus running?"),
        )
    })?;
    builder
        .method_timeout(DBUS_METHOD_TIMEOUT)
        .build()
        .map_err(|error| {
            dbus_error(
                "connecting to the system D-Bus",
                format_args!("{error}; is the system bus running?"),
            )
        })
}

fn daemon_proxy<'a>(connection: &'a Connection) -> CliResult<Proxy<'a>> {
    Proxy::new(connection, DBUS_DESTINATION, DBUS_PATH, DBUS_INTERFACE)
        .map_err(|error| dbus_error("creating the aorusd proxy", error))
}

fn call_status(proxy: &Proxy<'_>) -> CliResult<HashMap<String, OwnedValue>> {
    proxy
        .call("GetStatus", &())
        .map_err(|error| dbus_error("GetStatus", error))
}

fn value_text(value: &OwnedValue) -> String {
    if let Ok(value) = String::try_from(value.clone()) {
        return value;
    }
    if let Ok(value) = bool::try_from(value) {
        return value.to_string();
    }
    if let Ok(value) = u8::try_from(value) {
        return value.to_string();
    }
    if let Ok(value) = u32::try_from(value) {
        return value.to_string();
    }
    if let Ok(value) = i32::try_from(value) {
        return value.to_string();
    }
    format!("{value:?}")
}

fn status_u8(status: &HashMap<String, OwnedValue>, key: &str) -> Option<u8> {
    status.get(key).and_then(|value| u8::try_from(value).ok())
}

fn status_i32(status: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    status.get(key).and_then(|value| i32::try_from(value).ok())
}

fn status_bool(status: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    status.get(key).and_then(|value| bool::try_from(value).ok())
}

fn status_u8_array(status: &HashMap<String, OwnedValue>, key: &str) -> Option<Vec<u8>> {
    status
        .get(key)
        .and_then(|value| Vec::<u8>::try_from(value.clone()).ok())
}

fn fan_mode_name(mode: u8) -> &'static str {
    match mode {
        0 => "normal",
        1 => "silent",
        2 => "gaming",
        3 => "custom",
        4 => "auto",
        5 => "fixed",
        _ => "unknown",
    }
}

fn print_status(status: &HashMap<String, OwnedValue>) {
    let mut ordered = BTreeMap::new();
    for (key, value) in status {
        ordered.insert(key, value);
    }

    for (key, value) in ordered {
        if let Some(millicelsius) = status_i32(status, key)
            && key.starts_with("temp")
            && key.ends_with("_millicelsius")
        {
            println!(
                "{key}={} ({:.1} °C)",
                millicelsius,
                millicelsius as f64 / 1000.0
            );
            continue;
        }
        if key == "fan_mode"
            && let Some(mode) = status_u8(status, key)
        {
            println!("{key}={mode} ({})", fan_mode_name(mode));
            continue;
        }
        if matches!(key.as_str(), "cap_fan_modes" | "cap_gpu_boost_values")
            && let Some(values) = status_u8_array(status, key)
        {
            let values = values
                .into_iter()
                .map(|value| {
                    if key == "cap_fan_modes" {
                        format!("{value} ({})", fan_mode_name(value))
                    } else {
                        value.to_string()
                    }
                })
                .collect::<Vec<_>>();
            println!("{key}=[{}]", values.join(", "));
            continue;
        }
        println!("{key}={}", value_text(value));
    }
}

fn show_status(proxy: &Proxy<'_>) -> CliResult<()> {
    let status = call_status(proxy)?;
    if status.is_empty() {
        return Err(CliError::new(
            EXIT_OPERATION,
            "GetStatus returned no keys; the daemon/API version is incompatible",
        ));
    }
    print_status(&status);
    Ok(())
}

fn show_curve(proxy: &Proxy<'_>) -> CliResult<()> {
    let curve: Vec<(u8, u8)> = proxy
        .call("GetFanCurve", &())
        .map_err(|error| dbus_error("GetFanCurve", error))?;
    if curve.len() != FAN_CURVE_POINTS {
        return Err(CliError::new(
            EXIT_OPERATION,
            format!(
                "GetFanCurve returned {} points; expected exactly {FAN_CURVE_POINTS}",
                curve.len()
            ),
        ));
    }

    println!("index temperature_c raw_speed speed_percent");
    for (index, (temperature, raw_speed)) in curve.iter().copied().enumerate() {
        println!(
            "{index:>5} {temperature:>14} {raw_speed:>9} {:>13.1}",
            f64::from(raw_speed) * 100.0 / 255.0
        );
    }
    Ok(())
}

fn parse_u16(value: &str, description: &str) -> CliResult<u16> {
    value.parse::<u16>().map_err(|error| {
        CliError::new(
            EXIT_USAGE,
            format!("invalid {description} '{value}': expected a non-negative integer ({error})"),
        )
    })
}

fn parse_curve_point(value: &str, index: usize) -> CliResult<FanPoint> {
    let (temperature, raw_speed) = value.split_once(':').ok_or_else(|| {
        CliError::new(
            EXIT_USAGE,
            format!(
                "invalid curve point {} '{value}': expected TEMP:RAW_SPEED",
                index + 1
            ),
        )
    })?;
    if temperature.is_empty() || raw_speed.is_empty() || raw_speed.contains(':') {
        return Err(CliError::new(
            EXIT_USAGE,
            format!(
                "invalid curve point {} '{value}': expected one TEMP:RAW_SPEED pair",
                index + 1
            ),
        ));
    }
    let temperature = parse_u16(temperature, "curve temperature")?;
    if temperature > 100 {
        return Err(CliError::new(
            EXIT_USAGE,
            format!(
                "curve point {} temperature {temperature} is outside the allowed 0-100°C range",
                index + 1
            ),
        ));
    }
    let raw_speed = parse_u16(raw_speed, "raw fan level")?;
    if raw_speed > u16::from(u8::MAX) {
        return Err(CliError::new(
            EXIT_USAGE,
            format!(
                "curve point {} raw fan level {raw_speed} is outside the allowed 0-255 range",
                index + 1
            ),
        ));
    }
    Ok(FanPoint::new(temperature as u8, raw_speed as u8))
}

fn parse_curve(point_arguments: &[String]) -> CliResult<FanCurve> {
    if point_arguments.len() != FAN_CURVE_POINTS {
        return Err(CliError::new(
            EXIT_USAGE,
            format!(
                "curve apply requires exactly {FAN_CURVE_POINTS} TEMP:RAW_SPEED points, got {}",
                point_arguments.len()
            ),
        ));
    }
    let points = point_arguments
        .iter()
        .enumerate()
        .map(|(index, value)| parse_curve_point(value, index))
        .collect::<CliResult<Vec<_>>>()?;
    FanCurve::new(points).map_err(|error| {
        CliError::new(
            EXIT_USAGE,
            format!(
                "invalid fan curve: {error}; temperatures and raw fan levels must not decrease"
            ),
        )
    })
}

fn apply_curve(proxy: &Proxy<'_>, curve: &FanCurve) -> CliResult<()> {
    let status = call_status(proxy)?;
    if status_u8(&status, "cap_fan_curve_points") != Some(FAN_CURVE_POINTS as u8) {
        return Err(CliError::new(
            EXIT_VALIDATION,
            format!(
                "the daemon does not advertise an available {FAN_CURVE_POINTS}-point fan curve; refusing the write"
            ),
        ));
    }
    let wire: Vec<(u8, u8)> = curve
        .points()
        .iter()
        .map(|point| (point.temperature, point.raw_speed))
        .collect();
    let _: () = proxy
        .call("SetFanCurve", &wire)
        .map_err(|error| dbus_error("SetFanCurve", error))?;
    println!(
        "requested validated {FAN_CURVE_POINTS}-point fan curve; aorusd will apply, verify, and select the profile"
    );
    Ok(())
}

fn set_profile(proxy: &Proxy<'_>, profile: &str) -> CliResult<()> {
    if !matches!(profile, "performance" | "balanced" | "battery") {
        return Err(CliError::new(
            EXIT_USAGE,
            format!("unknown power profile '{profile}'; use performance, balanced, or battery"),
        ));
    }
    let profile = profile.to_owned();
    let _: () = proxy
        .call("SetPowerProfile", &profile)
        .map_err(|error| dbus_error("SetPowerProfile", error))?;
    println!("requested power profile: {profile}");
    Ok(())
}

fn set_fan_mode(proxy: &Proxy<'_>, mode: &str) -> CliResult<()> {
    let value = fan_mode_value(mode).ok_or_else(|| {
        CliError::new(
            EXIT_USAGE,
            format!("unknown fan profile '{mode}'; use normal, silent, gaming, or custom"),
        )
    })?;
    let status = call_status(proxy)?;
    let supported = status_u8_array(&status, "cap_fan_modes").unwrap_or_default();
    if !supported.contains(&value) {
        return Err(CliError::new(
            EXIT_VALIDATION,
            format!(
                "fan profile '{mode}' is not advertised as available by the daemon; refusing the write"
            ),
        ));
    }
    if value == 3 && status_bool(&status, "custom_curve_available") != Some(true) {
        return Err(CliError::new(
            EXIT_VALIDATION,
            "custom fan profile is unavailable: the daemon does not report a stored, validated custom curve",
        ));
    }
    let _: () = proxy
        .call("SetFanMode", &value)
        .map_err(|error| dbus_error("SetFanMode", error))?;
    println!("requested fan profile: {mode}");
    Ok(())
}

fn reapply(proxy: &Proxy<'_>) -> CliResult<()> {
    let _: () = proxy
        .call("ReapplyFanProfile", &())
        .map_err(|error| dbus_error("ReapplyFanProfile", error))?;
    println!("reapplied the mapped firmware fan profile");
    Ok(())
}

fn fan_mode_value(mode: &str) -> Option<u8> {
    match mode.to_ascii_lowercase().as_str() {
        "normal" => Some(0),
        "silent" => Some(1),
        "gaming" => Some(2),
        "custom" => Some(3),
        _ => None,
    }
}

fn show_mappings(proxy: &Proxy<'_>) -> CliResult<()> {
    let mappings: HashMap<String, u8> = proxy
        .call("GetProfileMappings", &())
        .map_err(|error| dbus_error("GetProfileMappings", error))?;
    if mappings.is_empty() {
        return Err(CliError::new(
            EXIT_OPERATION,
            "GetProfileMappings returned no mappings; the daemon/API version is incompatible",
        ));
    }
    let mut ordered = BTreeMap::new();
    for (profile, mode) in mappings {
        ordered.insert(profile, mode);
    }
    for (profile, mode) in ordered {
        println!("{profile}={mode} ({})", fan_mode_name(mode));
    }
    Ok(())
}

fn parse_mapping(value: &str) -> CliResult<(String, u8)> {
    let (profile, mode) = value.split_once('=').ok_or_else(|| {
        CliError::new(
            EXIT_USAGE,
            format!("invalid mapping '{value}': expected PROFILE=FAN_PROFILE"),
        )
    })?;
    let profile = profile.trim().to_ascii_lowercase();
    if !matches!(profile.as_str(), "performance" | "balanced" | "battery") {
        return Err(CliError::new(
            EXIT_USAGE,
            format!("unknown power profile '{profile}'; use performance, balanced, or battery"),
        ));
    }
    let mode_name = mode.trim().to_ascii_lowercase();
    let mode_value = fan_mode_value(&mode_name).ok_or_else(|| {
        CliError::new(
            EXIT_USAGE,
            format!("unknown fan profile '{mode}'; use normal, silent, gaming, or custom"),
        )
    })?;
    Ok((profile, mode_value))
}

fn set_mappings(proxy: &Proxy<'_>, mapping_arguments: &[String]) -> CliResult<()> {
    if mapping_arguments.is_empty() {
        return Err(CliError::new(
            EXIT_USAGE,
            "mappings set requires at least one PROFILE=FAN_PROFILE pair",
        ));
    }
    let mut mappings = HashMap::new();
    for argument in mapping_arguments {
        let (profile, mode) = parse_mapping(argument)?;
        if mappings.insert(profile.clone(), mode).is_some() {
            return Err(CliError::new(
                EXIT_USAGE,
                format!("duplicate mapping for power profile '{profile}'"),
            ));
        }
    }
    let status = call_status(proxy)?;
    let supported_modes = status_u8_array(&status, "cap_fan_modes").unwrap_or_default();
    for mode in mappings.values().copied() {
        if !supported_modes.contains(&mode) {
            return Err(CliError::new(
                EXIT_VALIDATION,
                format!(
                    "fan profile '{}' is not advertised as available by the daemon; refusing the mapping",
                    fan_mode_name(mode)
                ),
            ));
        }
    }
    if mappings.values().any(|mode| *mode == 3)
        && status_bool(&status, "custom_curve_available") != Some(true)
    {
        return Err(CliError::new(
            EXIT_VALIDATION,
            "cannot map a profile to custom: the daemon does not report a stored, validated custom curve",
        ));
    }
    let _: () = proxy
        .call("SetProfileMappings", &mappings)
        .map_err(|error| dbus_error("SetProfileMappings", error))?;
    let mut ordered = BTreeMap::new();
    for (profile, mode) in mappings {
        ordered.insert(profile, mode);
    }
    for (profile, mode) in ordered {
        println!("requested mapping: {profile} -> {}", fan_mode_name(mode));
    }
    Ok(())
}

fn require_capability(
    status: &HashMap<String, OwnedValue>,
    key: &str,
    control_name: &str,
) -> CliResult<()> {
    if status_bool(status, key) == Some(true) {
        Ok(())
    } else {
        Err(CliError::new(
            EXIT_VALIDATION,
            format!(
                "{control_name} is not advertised as available by the daemon; refusing the write"
            ),
        ))
    }
}

fn parse_charge_mode(value: &str) -> CliResult<u8> {
    let mode = match value.to_ascii_lowercase().as_str() {
        "0" | "normal" => 0,
        "1" | "custom" => 1,
        _ => {
            return Err(CliError::new(
                EXIT_USAGE,
                format!("invalid charge mode '{value}'; use 0|normal or 1|custom"),
            ));
        }
    };
    Ok(mode)
}

fn parse_charge_limit(value: &str) -> CliResult<u8> {
    let limit = parse_u16(value, "charge limit")?;
    if !(60..=100).contains(&limit) {
        return Err(CliError::new(
            EXIT_USAGE,
            format!("charge limit {limit} is outside the allowed 60-100% range"),
        ));
    }
    Ok(limit as u8)
}

fn parse_gpu_boost(value: &str) -> CliResult<u8> {
    let boost = parse_u16(value, "GPU boost value")?;
    if boost > u16::from(u8::MAX) {
        return Err(CliError::new(
            EXIT_USAGE,
            format!("GPU boost value {boost} is outside the D-Bus byte range 0-255"),
        ));
    }
    Ok(boost as u8)
}

fn set_charge_mode(proxy: &Proxy<'_>, value: &str) -> CliResult<()> {
    let mode = parse_charge_mode(value)?;
    let status = call_status(proxy)?;
    require_capability(&status, "cap_charge_mode", "charge mode")?;
    let _: () = proxy
        .call("SetChargeMode", &mode)
        .map_err(|error| dbus_error("SetChargeMode", error))?;
    println!(
        "requested charge mode {mode} ({})",
        if mode == 0 { "normal" } else { "custom" }
    );
    Ok(())
}

fn set_charge_limit(proxy: &Proxy<'_>, value: &str) -> CliResult<()> {
    let limit = parse_charge_limit(value)?;
    let status = call_status(proxy)?;
    require_capability(&status, "cap_charge_limit", "charge limit")?;
    let _: () = proxy
        .call("SetChargeLimit", &limit)
        .map_err(|error| dbus_error("SetChargeLimit", error))?;
    println!("requested charge limit: {limit}%");
    Ok(())
}

fn set_gpu_boost(proxy: &Proxy<'_>, value: &str) -> CliResult<()> {
    let boost = parse_gpu_boost(value)?;
    let status = call_status(proxy)?;
    let supported = status_u8_array(&status, "cap_gpu_boost_values").unwrap_or_default();
    if supported.is_empty() {
        return Err(CliError::new(
            EXIT_VALIDATION,
            "the daemon advertises no verified GPU boost values for this model; refusing to guess one",
        ));
    }
    if !supported.contains(&boost) {
        let values = supported
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(CliError::new(
            EXIT_VALIDATION,
            format!(
                "GPU boost value {boost} is not among the daemon-advertised verified values: {values}"
            ),
        ));
    }
    let _: () = proxy
        .call("SetGpuBoost", &boost)
        .map_err(|error| dbus_error("SetGpuBoost", error))?;
    println!("requested verified GPU boost value: {boost}");
    Ok(())
}

fn diagnostics(connection: &Connection) -> CliResult<()> {
    println!("bus=system");
    println!("destination={DBUS_DESTINATION}");
    println!("path={DBUS_PATH}");
    println!("interface={DBUS_INTERFACE}");

    let bus = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .map_err(|error| dbus_error("creating the D-Bus daemon proxy", error))?;
    let owner: bool = bus
        .call("NameHasOwner", &DBUS_DESTINATION)
        .map_err(|error| dbus_error("checking aorusd ownership", error))?;
    println!("daemon_name_owned={owner}");
    if !owner {
        return Err(CliError::new(
            EXIT_UNAVAILABLE,
            "aorusd is not running; install/start the shadow service before retrying",
        ));
    }

    let proxy = daemon_proxy(connection)?;
    let status = call_status(&proxy)?;
    println!("status_keys={}", status.len());
    print_status(&status);
    Ok(())
}

fn run(arguments: &[String]) -> CliResult<()> {
    if arguments.is_empty() {
        return Err(CliError::new(EXIT_USAGE, usage()));
    }
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "-h" | "--help") {
        println!("{}", usage());
        return Ok(());
    }

    let known_command = match arguments {
        [command] => matches!(command.as_str(), "status" | "diagnostics"),
        [command, subcommand] if command == "curve" => {
            matches!(subcommand.as_str(), "show" | "apply")
        }
        [command, subcommand, _points @ ..] if command == "curve" && subcommand == "apply" => true,
        [command, profile] if command == "profile" => {
            matches!(profile.as_str(), "performance" | "balanced" | "battery")
        }
        [command, mode] if command == "fan" => {
            matches!(
                mode.as_str(),
                "normal" | "silent" | "gaming" | "custom" | "reapply"
            )
        }
        [command, subcommand] if command == "mappings" => {
            matches!(subcommand.as_str(), "get" | "show" | "set")
        }
        [command, subcommand, mappings @ ..] if command == "mappings" && subcommand == "set" => {
            !mappings.is_empty()
        }
        [command, subcommand] if command == "mappings" && subcommand == "set" => true,
        [command, subcommand, value] if command == "charge" => {
            matches!(subcommand.as_str(), "mode" | "limit") && !value.is_empty()
        }
        [command, subcommand] if command == "gpu" && subcommand == "boost" => true,
        [command, subcommand, value] if command == "gpu" && subcommand == "boost" => {
            !value.is_empty()
        }
        _ => false,
    };
    if !known_command {
        return Err(CliError::new(EXIT_USAGE, usage()));
    }

    let parsed_curve = match arguments {
        [command, subcommand, points @ ..] if command == "curve" && subcommand == "apply" => {
            Some(parse_curve(points)?)
        }
        _ => None,
    };

    if let [command, subcommand, mappings @ ..] = arguments
        && command == "mappings"
        && subcommand == "set"
    {
        if mappings.is_empty() {
            return Err(CliError::new(
                EXIT_USAGE,
                "mappings set requires at least one PROFILE=FAN_PROFILE pair",
            ));
        }
        for mapping in mappings {
            parse_mapping(mapping)?;
        }
    }

    if let [command, subcommand] = arguments
        && command == "gpu"
        && subcommand == "boost"
    {
        return Err(CliError::new(
            EXIT_USAGE,
            "gpu boost requires a value advertised by the daemon",
        ));
    }

    if let [command, subcommand, value] = arguments {
        if command == "charge" && subcommand == "mode" {
            parse_charge_mode(value)?;
        } else if command == "charge" && subcommand == "limit" {
            parse_charge_limit(value)?;
        } else if command == "gpu" && subcommand == "boost" {
            parse_gpu_boost(value)?;
        }
    }

    let connection = system_connection()?;
    if matches!(arguments, [command] if command == "diagnostics") {
        return diagnostics(&connection);
    }
    let proxy = daemon_proxy(&connection)?;

    match arguments {
        [command] if command == "status" => show_status(&proxy),
        [command, subcommand] if command == "curve" && subcommand == "show" => show_curve(&proxy),
        [command, subcommand, _points @ ..] if command == "curve" && subcommand == "apply" => {
            apply_curve(
                &proxy,
                parsed_curve.as_ref().expect("curve was parsed above"),
            )
        }
        [command, profile] if command == "profile" => set_profile(&proxy, profile),
        [command, mode] if command == "fan" && mode == "reapply" => reapply(&proxy),
        [command, mode] if command == "fan" => set_fan_mode(&proxy, mode),
        [command, subcommand]
            if command == "mappings" && matches!(subcommand.as_str(), "get" | "show") =>
        {
            show_mappings(&proxy)
        }
        [command, subcommand, mappings @ ..] if command == "mappings" && subcommand == "set" => {
            set_mappings(&proxy, mappings)
        }
        [command, subcommand] if command == "mappings" && subcommand == "set" => {
            Err(CliError::new(
                EXIT_USAGE,
                "mappings set requires at least one PROFILE=FAN_PROFILE pair",
            ))
        }
        [command, subcommand, value] if command == "charge" && subcommand == "mode" => {
            set_charge_mode(&proxy, value)
        }
        [command, subcommand, value] if command == "charge" && subcommand == "limit" => {
            set_charge_limit(&proxy, value)
        }
        [command, subcommand, value] if command == "gpu" && subcommand == "boost" => {
            set_gpu_boost(&proxy, value)
        }
        [command, subcommand] if command == "gpu" && subcommand == "boost" => Err(CliError::new(
            EXIT_USAGE,
            "gpu boost requires a value advertised by the daemon",
        )),
        [command] if command == "diagnostics" => unreachable!("handled above"),
        _ => unreachable!("validated command shape above"),
    }
}

fn main() {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if let Err(error) = run(&arguments) {
        eprintln!("aorusctl: {}", error);
        process::exit(error.code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_curve_arguments() -> Vec<String> {
        (0..FAN_CURVE_POINTS)
            .map(|index| format!("{}:{}", index * 5, index * 10))
            .collect()
    }

    #[test]
    fn curve_parser_requires_exactly_fifteen_monotonic_points() {
        let arguments = valid_curve_arguments();
        assert!(parse_curve(&arguments).is_ok());

        let mut too_few = arguments.clone();
        too_few.pop();
        assert_eq!(
            parse_curve(&too_few).unwrap_err().code,
            EXIT_USAGE,
            "a partial curve must never reach D-Bus"
        );

        let mut descending = arguments;
        descending[4] = "10:40".to_owned();
        assert!(parse_curve(&descending).is_err());
    }

    #[test]
    fn curve_parser_rejects_out_of_range_values() {
        let mut arguments = valid_curve_arguments();
        arguments[0] = "101:0".to_owned();
        assert!(parse_curve(&arguments).is_err());
        arguments[0] = "0:256".to_owned();
        assert!(parse_curve(&arguments).is_err());
        arguments[0] = "-1:0".to_owned();
        assert!(parse_curve(&arguments).is_err());
    }

    #[test]
    fn mapping_parser_accepts_profiles_and_fan_profiles_only() {
        assert_eq!(
            parse_mapping("performance=gaming").unwrap(),
            ("performance".into(), 2)
        );
        assert!(parse_mapping("performance=fixed").is_err());
        assert!(parse_mapping("unknown=gaming").is_err());
        assert!(parse_mapping("performance").is_err());
    }
}
