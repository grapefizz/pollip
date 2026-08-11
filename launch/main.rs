#[allow(dead_code)]
#[path = "../config/mod.rs"]
mod config;
#[path = "../detection/mod.rs"]
mod detection;
#[path = "logging.rs"]
mod logging;
#[path = "../mods/mod.rs"]
mod mods;
#[path = "../nexus/mod.rs"]
mod nexus;
#[path = "../profiles/mod.rs"]
mod profiles;
#[path = "../platform/mod.rs"]
mod platform;
#[path = "../settings/mod.rs"]
mod settings;
#[path = "../thunderstore/mod.rs"]
mod thunderstore;
#[path = "../ui/mod.rs"]
mod ui;

use eframe::egui;
use nexus::enqueue_nxm_url;
use std::fs;
use std::path::PathBuf;
use ui::{apply_dark_theme, Shell};

struct Pollip {
    shell: Shell,
}

impl Default for Pollip {
    fn default() -> Self {
        Self {
            shell: Shell::default(),
        }
    }
}

impl eframe::App for Pollip {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.shell.draw(ui);
    }
}

fn main() -> eframe::Result {
    let mut queued_nxm = false;
    for arg in std::env::args().skip(1) {
        if arg.starts_with("nxm://") || arg.starts_with("NXM://") {
            match enqueue_nxm_url(&arg) {
                Ok(()) => queued_nxm = true,
                Err(error) => eprintln!("could not queue nexus download: {error}"),
            }
        }
    }

    if queued_nxm && peer_instance_running() {
        return Ok(());
    }

    write_instance_pid();

    let viewport = egui::ViewportBuilder::default()
        .with_title("pollip")
        .with_inner_size([1080.0, 720.0])
        .with_decorations(true);
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_title_shown(false)
        .with_titlebar_shown(false)
        .with_titlebar_buttons_shown(true);

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "pollip",
        native_options,
        Box::new(|cc| {
            apply_dark_theme(&cc.egui_ctx);
            match logging::init_logging() {
                Ok(path) => logging::info(format!("logging to {}", path.display())),
                Err(error) => eprintln!("could not start logging: {error}"),
            }
            Ok(Box::new(Pollip::default()))
        }),
    )
}

fn data_directory() -> Option<PathBuf> {
    platform::data_directory().ok()
}

fn instance_pid_path() -> Option<PathBuf> {
    Some(data_directory()?.join("instance.pid"))
}

fn peer_instance_running() -> bool {
    let Some(path) = instance_pid_path() else {
        return false;
    };
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        return false;
    };
    if pid == std::process::id() {
        return false;
    }
    platform::process_is_running(pid)
}

fn write_instance_pid() {
    let Some(path) = instance_pid_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, format!("{}\n", std::process::id()));
}
