use std::env;

fn main() {
    let prefix = env::var("AORUS_INSTALL_PREFIX").unwrap_or_else(|_| "/usr/local".to_owned());
    assert!(
        prefix.starts_with('/'),
        "AORUS_INSTALL_PREFIX must be absolute"
    );
    println!("cargo:rerun-if-env-changed=AORUS_INSTALL_PREFIX");
    println!("cargo:rustc-env=AORUS_CONTROL_PREFIX={prefix}");
}
