//! magnifier-settings — a small control panel for hypr-xp-magnifier.
//!
//! This doesn't talk to the magnifier process directly. It just reads and
//! writes the same settings.json file the magnifier watches and reloads a
//! few times a second, so changes here take effect shortly after moving a
//! slider or picking a position — no restart needed.

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

/// Which edge of the screen the bar is docked to.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum DockPosition {
    Top,
    Bottom,
    Left,
    Right,
}

impl Default for DockPosition {
    fn default() -> Self {
        DockPosition::Top
    }
}

fn default_thickness() -> u32 {
    200
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct Settings {
    zoom_factor: f64,
    #[serde(alias = "dock_height", default = "default_thickness")]
    thickness: u32,
    refresh_rate_hz: f64,
    sharpness: f64,
    #[serde(default)]
    position: DockPosition,
    #[serde(default)]
    follow_keypress: bool,
    #[serde(default)]
    focus_y_correction: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            zoom_factor: 2.5,
            thickness: 200,
            refresh_rate_hz: 60.0,
            sharpness: 1.0,
            position: DockPosition::Top,
            follow_keypress: false,
            focus_y_correction: 40,
        }
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("hypr-xp-magnifier").join("settings.json")
}

fn load_settings() -> Settings {
    match std::fs::read_to_string(config_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

fn save_settings(settings: &Settings) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings).unwrap_or_default();
    std::fs::write(path, text)
}

fn main() -> eframe::Result<()> {
    let mut settings = load_settings();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([400.0, 420.0]),
        ..Default::default()
    };

    eframe::run_simple_native("hypr-xp-magnifier settings", options, move |ctx, _frame| {
        let before = settings.clone();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("hypr-xp-magnifier");
            ui.label("Changes apply to the running magnifier within a fraction of a second.");
            ui.separator();

            ui.add(
                egui::Slider::new(&mut settings.zoom_factor, 1.0..=6.0)
                    .text("Zoom factor")
                    .fixed_decimals(2),
            );
            ui.add(
                egui::Slider::new(&mut settings.thickness, 40..=800).text("Thickness (px)"),
            );
            ui.add(
                egui::Slider::new(&mut settings.refresh_rate_hz, 10.0..=165.0)
                    .text("Refresh rate (Hz)")
                    .fixed_decimals(0),
            );
            ui.add(
                egui::Slider::new(&mut settings.sharpness, 0.0..=1.0)
                    .text("Sharpness (0 = smooth, 1 = sharp)")
                    .fixed_decimals(2),
            );

            ui.separator();
            ui.label("Docked to:");
            ui.horizontal(|ui| {
                ui.radio_value(&mut settings.position, DockPosition::Top, "Top");
                ui.radio_value(&mut settings.position, DockPosition::Bottom, "Bottom");
                ui.radio_value(&mut settings.position, DockPosition::Left, "Left");
                ui.radio_value(&mut settings.position, DockPosition::Right, "Right");
            });

            ui.separator();
            ui.checkbox(&mut settings.follow_keypress, "Follow keyboard focus (Tab, etc.)");
            ui.add(
                egui::Slider::new(&mut settings.focus_y_correction, -100..=100)
                    .text("Focus position correction (Y, px)"),
            );
            ui.small(
                "If focus-follow lands on the wrong row/field, nudge this until it lines up. \
                 Needs the AT-SPI accessibility bus running, and only works in apps that \
                 publish accessibility info (most GTK/Qt/Electron/browser apps do).",
            );

            ui.separator();
            ui.small(format!("Saved to {}", config_path().display()));
        });

        if settings != before {
            if let Err(err) = save_settings(&settings) {
                eprintln!("Failed to save settings: {err}");
            }
        }
    })
}
