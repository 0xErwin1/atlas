fn main() -> Result<(), Box<dyn std::error::Error>> {
    let webkitgtk = pkg_config::Config::new().probe("webkit2gtk-4.1")?;
    println!(
        "cargo:rustc-env=ATLAS_WEBKITGTK_VERSION={}",
        webkitgtk.version
    );
    tauri_build::build();
    Ok(())
}
