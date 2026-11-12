//! The few things that differ between Linux and Windows: opening files and
//! folders, what the bin for deleted files is called, fallback fonts and the
//! hints shown when Bluetooth or the firewall gets in the way.

use std::path::{Path, PathBuf};

/// Where deleted rides go, as the desktop calls it.
#[cfg(windows)]
pub const TRASH: &str = "Recycle Bin";
#[cfg(not(windows))]
pub const TRASH: &str = "Trash";

/// Open a file or folder with the desktop's default application.
pub fn open(path: impl AsRef<Path>) {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(not(windows))]
    let program = "xdg-open";
    let _ = std::process::Command::new(program)
        .arg(path.as_ref())
        .spawn();
}

/// Fonts with arrows, check marks and dots that egui's own fonts lack, most
/// suitable first. Missing files are skipped by the caller.
#[cfg(windows)]
pub fn fallback_fonts() -> Vec<PathBuf> {
    let fonts = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("Fonts");
    ["seguisym.ttf", "segoeui.ttf", "arial.ttf"]
        .iter()
        .map(|f| fonts.join(f))
        .collect()
}

#[cfg(not(windows))]
pub fn fallback_fonts() -> Vec<PathBuf> {
    // Ask fontconfig first: it knows the distribution's font layout.
    let fc = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", "sans-serif:charset=2190 2193 2714 25b6"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .filter(|p| p.ends_with(".ttf") || p.ends_with(".otf"));
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
        "/usr/share/fonts/gnu-free/FreeSans.otf",
        "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
    ];
    fc.into_iter()
        .map(PathBuf::from)
        .chain(CANDIDATES.iter().map(PathBuf::from))
        .collect()
}

/// Added to the error when no Bluetooth adapter can be opened.
#[cfg(windows)]
pub const BLUETOOTH_HINT: &str =
    "Is Bluetooth turned on in Windows Settings → Bluetooth & devices?";
#[cfg(not(windows))]
pub const BLUETOOTH_HINT: &str = "Is bluetoothd running and the adapter powered on?";

/// What to try when a scan finds nothing.
#[cfg(windows)]
pub const SCAN_HINT: &str = "• If scanning finds nothing: check that Bluetooth is on in Windows Settings → Bluetooth & devices.";
#[cfg(not(windows))]
pub const SCAN_HINT: &str =
    "• If scanning finds nothing: `systemctl status bluetooth` and `bluetoothctl power on`.";

/// Title of the device tips section.
#[cfg(windows)]
pub const TIPS_TITLE: &str = "KICKR SNAP / Windows tips";
#[cfg(not(windows))]
pub const TIPS_TITLE: &str = "KICKR SNAP / Linux tips";

/// How to let the phone remote through the firewall: an explanation and a
/// command to copy.
#[cfg(windows)]
pub fn firewall_hint(port: u16) -> (&'static str, String) {
    (
        "Check that the phone is on the same Wi-Fi. If it is, Windows Firewall may be \
         blocking it; allow the port in a terminal opened with \"Run as administrator\":",
        format!(
            "netsh advfirewall firewall add rule name=\"VeloForge remote\" dir=in action=allow protocol=TCP localport={port}"
        ),
    )
}

#[cfg(not(windows))]
pub fn firewall_hint(port: u16) -> (&'static str, String) {
    (
        "Check that the phone is on the same Wi-Fi. If it is, the firewall may be \
         blocking it; allow the port with:",
        format!(
            "sudo firewall-cmd --add-port={port}/tcp --permanent && sudo firewall-cmd --reload"
        ),
    )
}
