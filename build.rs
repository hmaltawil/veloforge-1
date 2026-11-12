//! On Windows, give veloforge.exe its icon and the name and version that
//! Explorer, Task Manager and the file's Properties show. Nothing to do on
//! other platforms.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=packaging/icons/veloforge.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("packaging/icons/veloforge.ico")
        .set("FileDescription", "VeloForge")
        .set("ProductName", "VeloForge")
        .set("OriginalFilename", "veloforge.exe")
        .set(
            "LegalCopyright",
            "Copyright (c) 2026 hmaltawil. MIT License.",
        );
    // A missing resource compiler only costs the icon, not the build.
    if let Err(e) = res.compile() {
        println!("cargo:warning=veloforge.exe will have no icon: {e}");
    }
}
