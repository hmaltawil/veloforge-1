// A windowed app: don't open a console window next to it on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod activity;
mod app;
mod charts;
mod devices;
mod gfx;
mod icu;
mod plan;
mod platform;
mod progress;
mod protocol;
mod remote;
mod ride;
mod route;
mod sound;
mod store;
mod theme;
mod workout;
mod world;

/// Window configurations to try: (MSAA samples, depth bits). Some GL drivers
/// lack multisampling or depth buffers; winit cannot create a second event
/// loop in-process, so each fallback relaunches the program.
const ATTEMPTS: [(u16, u8); 3] = [(4, 24), (0, 24), (0, 0)];

fn run(attempt: usize) -> eframe::Result {
    let (msaa, depth) = ATTEMPTS[attempt];
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("VeloForge")
            .with_app_id("veloforge")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(std::sync::Arc::new(eframe::egui::IconData {
                rgba: include_bytes!("../packaging/icons/veloforge-64.rgba").to_vec(),
                width: 64,
                height: 64,
            })),
        multisampling: msaa,
        depth_buffer: depth,
        ..Default::default()
    };
    eframe::run_native(
        "VeloForge",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, depth > 0)))),
    )
}

fn main() -> eframe::Result {
    let mut attempt: usize = std::env::var("VELOFORGE_GL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .min(ATTEMPTS.len() - 1);
    if attempt == 0 && std::env::var("VELOFORGE_MSAA").is_ok_and(|v| v == "0") {
        attempt = 1;
    }
    match run(attempt) {
        Err(e) if attempt + 1 < ATTEMPTS.len() => {
            eprintln!("veloforge: {e}; retrying with simpler graphics settings");
            let exe = std::env::current_exe().expect("current executable");
            let status = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .env("VELOFORGE_GL", (attempt + 1).to_string())
                .status()
                .expect("relaunch");
            std::process::exit(status.code().unwrap_or(1));
        }
        r => r,
    }
}
