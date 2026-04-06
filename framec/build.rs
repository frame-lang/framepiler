use std::env;
use std::path::Path;

fn main() {
    let frame_version =
        env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION must be set by Cargo");
    println!("cargo:rustc-env=FRAME_VERSION={}", frame_version);

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let project_root = Path::new(&manifest_dir).parent().unwrap();

    println!(
        "cargo:rerun-if-changed={}",
        project_root.join("Cargo.toml").display()
    );
}
