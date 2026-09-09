use std::{io, thread};

use aorus_control::{DBUS_DESTINATION, DBUS_PATH, dbus::AorusControl};
use zbus::blocking::connection::Builder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let write_enabled = match arguments.as_slice() {
        [] => true,
        [argument] if argument == "--shadow" => false,
        // Keep accepting the old explicit flag so upgrades cannot strand a
        // machine with an existing systemd override.
        [argument] if argument == "--write-enabled" => true,
        [argument] if argument == "--help" || argument == "-h" => {
            println!("Usage: aorusd [--shadow]");
            println!("Default mode is write-enabled; --shadow disables hardware mutations.");
            return Ok(());
        }
        _ => {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "usage: aorusd [--shadow]").into(),
            );
        }
    };

    let control = AorusControl::new(write_enabled).map_err(io::Error::other)?;
    eprintln!(
        "aorusd: starting in {} mode",
        if write_enabled {
            "write-enabled"
        } else {
            "shadow"
        }
    );
    let connection = Builder::system()?
        .name(DBUS_DESTINATION)?
        .serve_at(DBUS_PATH, control.clone())?
        .build()?;
    aorus_control::dbus::spawn_background_workers(control);
    eprintln!("aorusd: serving {DBUS_DESTINATION} at {DBUS_PATH}");
    loop {
        thread::park();
        let _ = &connection;
    }
}
