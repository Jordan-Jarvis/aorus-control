use std::{io, thread};

use aorus_control::{DBUS_DESTINATION, DBUS_PATH, dbus::AorusControl};
use zbus::blocking::connection::Builder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let write_enabled = match arguments.as_slice() {
        [] => false,
        [argument] if argument == "--write-enabled" => true,
        [argument] if argument == "--help" || argument == "-h" => {
            println!("Usage: aorusd [--write-enabled]");
            println!("Default mode is shadow; Python remains the authoritative fan writer.");
            return Ok(());
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: aorusd [--write-enabled]",
            )
            .into());
        }
    };
    if write_enabled && aorus_control::dbus::python_service_active().map_err(io::Error::other)? {
        return Err(io::Error::other(
            "refusing write-enabled startup while aorus-power-profile-sync.service is active",
        )
        .into());
    }

    let control = AorusControl::new(write_enabled).map_err(io::Error::other)?;
    eprintln!(
        "aorusd: starting in {} mode; Python service remains untouched",
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
