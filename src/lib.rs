//! Shared hardware, configuration, and D-Bus support for AORUS Control.

pub mod config;
pub mod curve;
pub mod dbus;
pub mod fn_buttons;
pub mod hardware;
pub mod hotkeys;
pub mod model;
pub mod native_keys;
pub mod profile;

pub const DBUS_DESTINATION: &str = "io.github.aoruslinux.Control1";
pub const DBUS_INTERFACE: &str = "io.github.aoruslinux.Control1";
pub const DBUS_PATH: &str = "/io/github/aoruslinux/Control1";
