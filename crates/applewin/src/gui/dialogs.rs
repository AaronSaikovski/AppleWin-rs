//! Modal dialogs (reboot confirmation, About) and the native file-picker
//! helpers used to load disk / HDD images.

use super::*;

impl EmulatorApp {
    pub(super) fn show_reboot_dialog(&mut self, ctx: &egui::Context) {
        // ── Confirm reboot dialog ─────────────────────────────────────────
        if let Some(power_cycle) = self.pending_reset {
            let mut do_reset = false;
            let mut do_cancel = false;
            let label = if power_cycle {
                "Hard Reset (power cycle)"
            } else {
                "Reset"
            };
            egui::Window::new("Confirm Reset")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label(format!("Are you sure you want to {label}?"));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("  OK  ").clicked() {
                            do_reset = true;
                        }
                        if ui.button("Cancel").clicked() {
                            do_cancel = true;
                        }
                    });
                });
            if do_reset {
                self.reset(power_cycle);
                self.pending_reset = None;
            }
            if do_cancel {
                self.pending_reset = None;
            }
        }
    }

    pub(super) fn show_about_dialog(&mut self, ctx: &egui::Context) {
        // ── About dialog ──────────────────────────────────────────────────
        if self.show_about {
            egui::Window::new("About AppleWin-rs")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("AppleWin-rs");
                        ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                        ui.add_space(4.0);
                        ui.label("Cross-platform Apple II emulator");
                        ui.label("Rust rewrite of AppleWin");
                        ui.add_space(8.0);
                        if ui.button("  OK  ").clicked() {
                            self.show_about = false;
                        }
                    });
                });
        }
    }
}

// ── File dialogs ────────────────────────────────────────────────────────────

pub(super) fn open_disk_dialog(title: &str, start_dir: Option<&str>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new()
        .set_title(title)
        .add_filter(
            "Apple II Disk Images",
            &[
                "dsk", "do", "po", "nib", "nb2", "woz", "hdv", "2mg", "2img", "img", "gz", "zip",
            ],
        )
        .add_filter("All Files", &["*"]);
    if let Some(dir) = start_dir {
        d = d.set_directory(dir);
    }
    d.pick_file()
}

pub(super) fn open_hdd_dialog(title: &str, start_dir: Option<&str>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new()
        .set_title(title)
        .add_filter(
            "Apple II HDD Images",
            &["hdv", "po", "2mg", "2img", "img", "gz", "zip"],
        )
        .add_filter("All Files", &["*"]);
    if let Some(dir) = start_dir {
        d = d.set_directory(dir);
    }
    d.pick_file()
}
