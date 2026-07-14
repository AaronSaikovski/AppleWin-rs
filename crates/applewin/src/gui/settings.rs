//! The tabbed Settings dialog and per-card options popups.

use super::*;

impl EmulatorApp {
    pub(super) fn show_settings_dialog(&mut self, ctx: &egui::Context) {
        // ── Settings dialog ───────────────────────────────────────────────
        if self.show_settings {
            let mut apply_settings = false;
            let mut cancel_settings = false;
            egui::Window::new("Settings")
                .collapsible(false)
                .resizable(false)
                .min_width(340.0)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    // ── Tab bar ───────────────────────────────────────────
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.settings_tab, 0, "Machine");
                        ui.selectable_value(&mut self.settings_tab, 1, "Video");
                        ui.selectable_value(&mut self.settings_tab, 2, "Audio");
                        ui.selectable_value(&mut self.settings_tab, 3, "Speed");
                        ui.selectable_value(&mut self.settings_tab, 4, "Input");
                        ui.selectable_value(&mut self.settings_tab, 5, "Slots");
                        ui.selectable_value(&mut self.settings_tab, 6, "Advanced");
                    });
                    ui.separator();

                    // ── Tab contents ──────────────────────────────────────
                    match self.settings_tab {
                        0 => self.render_machine_tab(ui),
                        1 => self.render_video_tab(ui),
                        2 => self.render_audio_tab(ui),
                        3 => self.render_speed_tab(ui),
                        4 => self.render_input_tab(ui),
                        5 => self.render_slots_tab(ui),
                        6 => self.render_advanced_tab(ui),
                        _ => {}
                    }
                    // ── Buttons ───────────────────────────────────────────
                    ui.add_space(8.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("  OK  ").clicked() {
                            apply_settings = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_settings = true;
                        }
                    });
                });

            if apply_settings {
                let machine_changed = self.pending_config.machine_type != self.config.machine_type
                    || self.pending_config.cpu_type != self.config.cpu_type;
                let slots_changed = self.pending_config.slot_cards != self.config.slot_cards;
                let scale_changed = self.pending_config.window_scale != self.config.window_scale;
                self.config = self.pending_config.clone();
                // Apply video settings immediately
                self.renderer.scanlines = self.config.scanlines;
                self.renderer.tv_mode = matches!(
                    self.config.video_type,
                    crate::config::VideoType::ColorTV | crate::config::VideoType::MonoTV
                );
                self.renderer.mono_tint = self.config.mono_tint();
                self.renderer.color_vertical_blend = self.config.color_vertical_blend;
                // Resize the window to match the new scale (no-op if maximized).
                if scale_changed {
                    self.resize_to_scale(ctx);
                }
                // Rebuild emulator if machine/CPU/slots changed
                if machine_changed || slots_changed {
                    let disk1 = self.disk1.clone();
                    let disk2 = self.disk2.clone();
                    self.emu = crate::make_emulator(
                        self.config.machine_type,
                        self.config.cpu_type,
                        &self.config.custom_rom_path,
                        &self.config.custom_f8_rom_path,
                    );
                    apply_slot_cards(&mut self.emu, &self.config);
                    self.disk_slot = self.config.disk2_slot();
                    self.emu.mode = apple2_core::emulator::AppMode::Running;
                    Self::reload_disk(&mut self.emu, self.disk_slot, 0, &disk1);
                    Self::reload_disk(&mut self.emu, self.disk_slot, 1, &disk2);
                    self.speaker_state = false;
                    self.dc_filter_ctr = 0;
                    self.spkr_cycle_rem = 0.0;
                    self.last_audio_cycle = self.emu.cpu.cycles;
                }
                self.config.save();
                self.show_settings = false;
            }
            if cancel_settings {
                self.pending_config = self.config.clone();
                self.show_settings = false;
            }
        }
    }

    pub(super) fn show_card_popups(&mut self, ctx: &egui::Context) {
        // ── Per-card options popups ───────────────────────────────────────
        for slot in 0..8usize {
            if !self.slot_options_open[slot] {
                continue;
            }
            let card_type = self.pending_config.slot_cards[slot];
            let title = format!("Slot {} — {} Options", slot, card_name(card_type));
            let mut still_open = self.slot_options_open[slot];
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .open(&mut still_open)
                .show(ctx, |ui| match card_type {
                    CardType::Mockingboard => {
                        ui.checkbox(
                            &mut self.pending_config.mockingboard_has_speech,
                            "Enable SSI263 speech chips",
                        );
                    }
                    CardType::Phasor => {
                        ui.label("Phasor mode:");
                        ui.radio_value(
                            &mut self.pending_config.phasor_native_mode,
                            true,
                            "Phasor native mode",
                        );
                        ui.radio_value(
                            &mut self.pending_config.phasor_native_mode,
                            false,
                            "Mockingboard compatible mode",
                        );
                    }
                    CardType::Saturn128K => {
                        ui.label("RAM size:");
                        for &kb in &[16u32, 32, 64, 128] {
                            ui.radio_value(
                                &mut self.pending_config.saturn_ram_kb,
                                kb,
                                format!("{kb}K"),
                            );
                        }
                    }
                    CardType::RamWorksIII => {
                        ui.label("RAM size:");
                        for &kb in &[64u32, 128, 256, 512, 1024, 2048, 4096, 8192] {
                            let label = if kb >= 1024 {
                                format!("{}MB", kb / 1024)
                            } else {
                                format!("{kb}K")
                            };
                            ui.radio_value(&mut self.pending_config.ramworks_ram_kb, kb, label);
                        }
                    }
                    _ => {
                        ui.label("No options available.");
                    }
                });
            self.slot_options_open[slot] = still_open;
        }
    }
}
