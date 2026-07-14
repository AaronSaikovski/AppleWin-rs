//! The top menu bar (File / Machine / View / Help).

use super::*;

impl EmulatorApp {
    pub(super) fn show_menu_bar(&mut self, ctx: &egui::Context, act: &mut DeferredActions) {
        // Snapshot disk state for use in closures (avoids borrow conflicts)
        let d1_loaded = self.disk1.is_some();
        let d2_loaded = self.disk2.is_some();

        // ── Menu bar ──────────────────────────────────────────────────────
        egui::TopBottomPanel::top("menubar")
            .frame(
                egui::Frame::none()
                    .fill(WIN_FACE)
                    .inner_margin(egui::Margin::symmetric(4.0, 2.0)),
            )
            .show(ctx, |ui| {
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        // ── Floppy disks ─────────────────────────────────
                        if ui.button("Load Disk 1…").clicked() {
                            act.load_disk1 = true;
                            ui.close_menu();
                        }
                        if ui.button("Load Disk 2…").clicked() {
                            act.load_disk2 = true;
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(d1_loaded, egui::Button::new("Eject Disk 1"))
                            .clicked()
                        {
                            act.eject_disk1 = true;
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(d2_loaded, egui::Button::new("Eject Disk 2"))
                            .clicked()
                        {
                            act.eject_disk2 = true;
                            ui.close_menu();
                        }
                        // ── Recent Disks submenu ──────────────────────────
                        ui.menu_button("Recent Disks", |ui| {
                            if self.config.recent_disks.is_empty() {
                                ui.label("(none)");
                            } else {
                                for path in &self.config.recent_disks {
                                    let name = std::path::Path::new(path)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned();
                                    if ui.button(name).clicked() {
                                        act.recent_disk = Some(path.clone());
                                        ui.close_menu();
                                    }
                                }
                            }
                        });
                        ui.separator();
                        // ── HDD images ───────────────────────────────────
                        {
                            let hdd1_name = self
                                .config
                                .last_hdd1
                                .as_deref()
                                .and_then(|p| std::path::Path::new(p).file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "(none)".to_string());
                            let hdd2_name = self
                                .config
                                .last_hdd2
                                .as_deref()
                                .and_then(|p| std::path::Path::new(p).file_name())
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "(none)".to_string());
                            let hdd1_loaded = self.config.last_hdd1.is_some();
                            let hdd2_loaded = self.config.last_hdd2.is_some();
                            if ui.button(format!("Load HDD 1…  [{}]", hdd1_name)).clicked() {
                                act.load_hdd1 = true;
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(hdd1_loaded, egui::Button::new("Eject HDD 1"))
                                .clicked()
                            {
                                act.eject_hdd1 = true;
                                ui.close_menu();
                            }
                            if ui.button(format!("Load HDD 2…  [{}]", hdd2_name)).clicked() {
                                act.load_hdd2 = true;
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(hdd2_loaded, egui::Button::new("Eject HDD 2"))
                                .clicked()
                            {
                                act.eject_hdd2 = true;
                                ui.close_menu();
                            }
                        }
                        // ── Recent HDDs submenu ───────────────────────────
                        ui.menu_button("Recent HDDs", |ui| {
                            if self.config.recent_hdds.is_empty() {
                                ui.label("(none)");
                            } else {
                                for path in &self.config.recent_hdds {
                                    let name = std::path::Path::new(path)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .into_owned();
                                    if ui.button(name).clicked() {
                                        act.recent_hdd = Some(path.clone());
                                        ui.close_menu();
                                    }
                                }
                            }
                        });
                        ui.separator();
                        if ui.button("Screenshot       F12").clicked() {
                            act.screenshot = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Exit").clicked() {
                            act.quit = true;
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Machine", |ui| {
                        if ui.button("Reset          Ctrl+F2").clicked() {
                            act.reset = true;
                            ui.close_menu();
                        }
                        if ui.button("Hard Reset          F1").clicked() {
                            act.hard_reset = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Settings…").clicked() {
                            act.show_settings = true;
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        let label = if self.fullscreen {
                            "Exit Fullscreen  F11"
                        } else {
                            "Fullscreen       F11"
                        };
                        if ui.button(label).clicked() {
                            act.fullscreen = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        // ── Video Mode submenu ────────────────────────────
                        ui.menu_button("Video Mode", |ui| {
                            let modes: &[(VideoType, &str)] = &[
                                (VideoType::ColorTV, "Color (NTSC TV)      Ctrl+1"),
                                (VideoType::ColorIdealized, "Color (Composite)    Ctrl+2"),
                                (VideoType::ColorRGB, "RGB                  Ctrl+3"),
                                (VideoType::MonoWhite, "Monochrome (white)   Ctrl+4"),
                                (VideoType::MonoGreen, "Monochrome (green)   Ctrl+5"),
                                (VideoType::MonoAmber, "Monochrome (amber)"),
                                (VideoType::MonoTV, "Monochrome TV"),
                                (VideoType::ColorMonitorNtsc, "Color Monitor NTSC"),
                                (VideoType::MonoCustom, "Monochrome (custom)"),
                            ];
                            let current_vt = self.config.video_type;
                            let mut chosen: Option<VideoType> = None;
                            for &(mode, label) in modes {
                                let text = if current_vt == mode {
                                    format!("✓ {label}")
                                } else {
                                    format!("  {label}")
                                };
                                if ui.button(text).clicked() {
                                    chosen = Some(mode);
                                    ui.close_menu();
                                }
                            }
                            if let Some(mode) = chosen {
                                self.config.video_type = mode;
                                self.renderer.tv_mode = matches!(
                                    mode,
                                    crate::config::VideoType::ColorTV
                                        | crate::config::VideoType::MonoTV
                                );
                                self.renderer.mono_tint = self.config.mono_tint();
                                self.config.save();
                            }
                        });
                        ui.separator();
                        // ── Display toggles ───────────────────────────────
                        if ui
                            .checkbox(&mut self.config.scanlines, "Scanlines")
                            .changed()
                        {
                            self.renderer.scanlines = self.config.scanlines;
                            self.config.save();
                        }
                        if ui
                            .checkbox(
                                &mut self.config.color_vertical_blend,
                                "Colour vertical blend",
                            )
                            .changed()
                        {
                            self.renderer.color_vertical_blend = self.config.color_vertical_blend;
                            self.config.save();
                        }
                        {
                            let mut is_50hz = self.config.video_refresh_hz == 50;
                            if ui.checkbox(&mut is_50hz, "50 Hz (PAL) mode").changed() {
                                self.config.video_refresh_hz = if is_50hz { 50 } else { 60 };
                                self.config.save();
                            }
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        if ui.button("About AppleWin-rs…").clicked() {
                            act.about = true;
                            ui.close_menu();
                        }
                    });
                });
            });
    }
}
