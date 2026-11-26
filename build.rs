//! Build the aero bike in when its baked model is there (see
//! `tools/bike-model`: the model isn't in the repository), and on Windows,
//! give veloforge.exe its icon and the name and version that Explorer,
//! Task Manager and the file's Properties show.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(aero_model)");
    println!("cargo:rerun-if-changed=assets/models");
    let baked = ["bike-aero.vfb", "bike-aero-far.vfb"];
    if baked
        .iter()
        .all(|f| std::path::Path::new("assets/models").join(f).exists())
    {
        println!("cargo:rustc-cfg=aero_model");
    }
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
