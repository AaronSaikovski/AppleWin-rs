//! The tabbed Settings dialog and per-card options popups.

use super::*;

impl EmulatorApp {
    pub(super) fn show_settings_dialog(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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
                            0 => {
                                // Machine tab
                                egui::Grid::new("machine_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label("Computer type:");
                                        egui::ComboBox::from_id_source("machine_type")
                                            .selected_text(model_name(self.pending_config.machine_type))
                                            .show_ui(ui, |ui| {
                                                for m in [
                                                    Apple2Model::AppleII,
                                                    Apple2Model::AppleIIPlus,
                                                    Apple2Model::AppleIIe,
                                                    Apple2Model::AppleIIeEnh,
                                                    Apple2Model::AppleIIc,
                                                    // Apple IIgs support temporarily disabled in UI.
                                                    // Apple2Model::AppleIIgs,
                                                ] {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.machine_type,
                                                        m,
                                                        model_name(m),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                        ui.label("CPU type:");
                                        if self.pending_config.machine_type == Apple2Model::AppleIIgs {
                                            // IIgs always uses 65C816
                                            self.pending_config.cpu_type = CpuType::Cpu65C816;
                                            ui.label("65C816 (built-in)");
                                        } else {
                                            // Apple IIc always uses 65C02.
                                            if self.pending_config.machine_type.is_iic() {
                                                self.pending_config.cpu_type = CpuType::Cpu65C02;
                                            }
                                            let cpu_enabled = !self.pending_config.machine_type.is_iic();
                                            ui.add_enabled_ui(cpu_enabled, |ui| {
                                                egui::ComboBox::from_id_source("cpu_type")
                                                    .selected_text(cpu_name(self.pending_config.cpu_type))
                                                    .show_ui(ui, |ui| {
                                                        for c in [CpuType::Cpu6502, CpuType::Cpu65C02, CpuType::CpuZ80] {
                                                            ui.selectable_value(
                                                                &mut self.pending_config.cpu_type,
                                                                c,
                                                                cpu_name(c),
                                                            );
                                                        }
                                                    });
                                            }); // add_enabled_ui
                                        }
                                        ui.end_row();

                                        // IIgs-specific settings
                                        if self.pending_config.machine_type == Apple2Model::AppleIIgs {
                                            ui.label("IIgs RAM:");
                                            egui::ComboBox::from_id_source("iigs_ram")
                                                .selected_text(format!("{} KB", self.pending_config.iigs_ram_kb))
                                                .show_ui(ui, |ui| {
                                                    for &kb in &[256u32, 512, 1024, 2048, 4096, 8192] {
                                                        let label = if kb >= 1024 {
                                                            format!("{} MB", kb / 1024)
                                                        } else {
                                                            format!("{} KB", kb)
                                                        };
                                                        ui.selectable_value(
                                                            &mut self.pending_config.iigs_ram_kb,
                                                            kb,
                                                            label,
                                                        );
                                                    }
                                                });
                                            ui.end_row();

                                            ui.label("IIgs ROM:");
                                            let rom_label = self.pending_config.iigs_rom_path
                                                .as_deref()
                                                .and_then(|p| std::path::Path::new(p).file_name())
                                                .and_then(|n| n.to_str())
                                                .unwrap_or("(auto-detect)");
                                            if ui.button(rom_label).clicked()
                                                && let Some(path) = rfd::FileDialog::new()
                                                    .add_filter("ROM files", &["bin", "rom", "ROM"])
                                                    .pick_file()
                                            {
                                                self.pending_config.iigs_rom_path =
                                                    Some(path.to_string_lossy().to_string());
                                            }
                                            ui.end_row();
                                        }
                                    });
                                if self.pending_config.machine_type != self.config.machine_type
                                    || self.pending_config.cpu_type != self.config.cpu_type
                                {
                                    ui.add_space(4.0);
                                    ui.colored_label(
                                        Color32::from_rgb(180, 100, 0),
                                        "⚠ Machine change requires a hard reset.",
                                    );
                                }
                            }
                            1 => {
                                // Video tab
                                egui::Grid::new("video_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label("Video type:");
                                        egui::ComboBox::from_id_source("video_type")
                                            .selected_text(video_type_name(self.pending_config.video_type))
                                            .show_ui(ui, |ui| {
                                                for &vt in ALL_VIDEO_TYPES {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.video_type,
                                                        vt,
                                                        video_type_name(vt),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                        // Monochrome custom colour picker
                                        if self.pending_config.video_type == VideoType::MonoCustom {
                                            ui.label("Mono colour:");
                                            let c = self.pending_config.monochrome_color;
                                            let mut rgb = [
                                                ((c >> 16) & 0xFF) as u8,
                                                ((c >>  8) & 0xFF) as u8,
                                                ( c        & 0xFF) as u8,
                                            ];
                                            if ui.color_edit_button_srgb(&mut rgb).changed() {
                                                self.pending_config.monochrome_color =
                                                    ((rgb[0] as u32) << 16) |
                                                    ((rgb[1] as u32) <<  8) |
                                                     (rgb[2] as u32);
                                            }
                                            ui.end_row();
                                        }
                                        ui.label("Refresh rate:");
                                        ui.horizontal(|ui| {
                                            ui.radio_value(&mut self.pending_config.video_refresh_hz, 60, "60 Hz (NTSC)");
                                            ui.radio_value(&mut self.pending_config.video_refresh_hz, 50, "50 Hz (PAL)");
                                        });
                                        ui.end_row();
                                    });
                                ui.add_space(4.0);
                                ui.checkbox(&mut self.pending_config.scanlines, "CRT scanlines (half-scanline darkening)");
                                ui.checkbox(&mut self.pending_config.color_vertical_blend, "Colour vertical blend");
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(format!(
                                        "Cycles per frame: {}",
                                        self.pending_config.cycles_per_frame()
                                    ))
                                    .small(),
                                );
                            }
                            2 => {
                                // Audio tab
                                ui.add_space(4.0);
                                ui.add(
                                    egui::Slider::new(&mut self.pending_config.master_volume, 0..=100)
                                        .text("Master volume")
                                        .suffix("%"),
                                );
                            }
                            3 => {
                                // Speed tab
                                const SPEED_NORMAL: u32 = 10; // 1.023 MHz
                                ui.add_space(4.0);
                                let mhz = self.pending_config.emulation_speed as f64 * 0.1023;
                                let speed_label = if self.pending_config.emulation_speed == SPEED_NORMAL {
                                    "CPU speed  (Authentic / Normal)".to_string()
                                } else {
                                    format!("CPU speed  ({:.2} MHz)", mhz)
                                };
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::Slider::new(&mut self.pending_config.emulation_speed, 1..=40)
                                            .text(speed_label),
                                    );
                                    if self.pending_config.emulation_speed != SPEED_NORMAL
                                        && ui.small_button("Reset to normal").clicked() {
                                        self.pending_config.emulation_speed = SPEED_NORMAL;
                                    }
                                });
                                ui.add_space(4.0);
                                ui.checkbox(
                                    &mut self.pending_config.enhanced_disk_speed,
                                    "Enhanced disk speed  (16× while motor is spinning)",
                                );
                                ui.add_space(4.0);
                                ui.label(RichText::new(
                                    "Speeds up disk-based game boot times significantly."
                                ).small());
                            }
                            4 => {
                                // Input tab
                                egui::Grid::new("input_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label("Joystick 1:");
                                        egui::ComboBox::from_id_source("joy0_type")
                                            .selected_text(joystick_type_name(self.pending_config.joystick0_type))
                                            .show_ui(ui, |ui| {
                                                for &jt in ALL_JOYSTICK_TYPES {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.joystick0_type,
                                                        jt,
                                                        joystick_type_name(jt),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                        ui.label("Joystick 2:");
                                        egui::ComboBox::from_id_source("joy1_type")
                                            .selected_text(joystick_type_name(self.pending_config.joystick1_type))
                                            .show_ui(ui, |ui| {
                                                for &jt in ALL_JOYSTICK_TYPES {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.joystick1_type,
                                                        jt,
                                                        joystick_type_name(jt),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                    });
                                ui.add_space(4.0);
                                ui.checkbox(&mut self.pending_config.joystick_swap_buttons,    "Swap joystick buttons");
                                ui.checkbox(&mut self.pending_config.joystick_autofire,         "Auto-fire button 0");
                                ui.checkbox(&mut self.pending_config.joystick_self_centering,   "Self-centring joystick");
                                ui.checkbox(&mut self.pending_config.joystick_cursor_control,   "Cursor keys control joystick");
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(4.0);
                                ui.checkbox(&mut self.pending_config.mouse_crosshair,           "Show crosshair mouse cursor");
                                ui.checkbox(&mut self.pending_config.mouse_restrict_to_window,  "Restrict mouse to window");
                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(4.0);
                                egui::Grid::new("paddle_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Paddle X trim:");
                                        let mut px = self.pending_config.paddle_x_trim as i32;
                                        if ui.add(egui::Slider::new(&mut px, -128..=127)).changed() {
                                            self.pending_config.paddle_x_trim = px as i8;
                                        }
                                        ui.end_row();
                                        ui.label("Paddle Y trim:");
                                        let mut py = self.pending_config.paddle_y_trim as i32;
                                        if ui.add(egui::Slider::new(&mut py, -128..=127)).changed() {
                                            self.pending_config.paddle_y_trim = py as i8;
                                        }
                                        ui.end_row();
                                    });
                                ui.add_space(4.0);
                                ui.label(RichText::new(
                                    "Physical gamepads, keyboard (Arrow/NumPad), and mouse modes supported."
                                ).small());
                            }
                            5 => {
                                // Slots tab
                                ui.add_space(4.0);
                                egui::Grid::new("slots_grid")
                                    .num_columns(3)
                                    .spacing([12.0, 4.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        if self.pending_config.machine_type.is_iic() {
                                            // Apple IIc: fixed built-in peripherals (read-only)
                                            let iic_slots = [
                                                "(empty)",            // slot 0
                                                "Serial (modem)",     // slot 1
                                                "Serial (printer)",   // slot 2
                                                "80-column (built-in)", // slot 3
                                                "Mouse",              // slot 4
                                                "(empty)",            // slot 5
                                                "Disk II",            // slot 6
                                                "(empty)",            // slot 7
                                            ];
                                            for (slot, name) in iic_slots.iter().enumerate() {
                                                ui.label(format!("Slot {slot}:"));
                                                ui.label(*name);
                                                ui.label(""); // spacer
                                                ui.end_row();
                                            }
                                            // No aux slot for IIc (128KB built-in)
                                            ui.label("Aux:");
                                            ui.label("128KB built-in");
                                            ui.label("");
                                        } else {
                                        for slot in 0..8usize {
                                            ui.label(format!("Slot {slot}:"));
                                            let current = self.pending_config.slot_cards[slot];
                                            egui::ComboBox::from_id_source(format!("slot_{slot}"))
                                                .selected_text(card_name(current))
                                                .show_ui(ui, |ui| {
                                                    for &card in IMPLEMENTED_CARDS {
                                                        ui.selectable_value(
                                                            &mut self.pending_config.slot_cards[slot],
                                                            card,
                                                            card_name(card),
                                                        );
                                                    }
                                                });
                                            // Options button — only for cards with configurable options
                                            let has_options = matches!(
                                                current,
                                                CardType::Mockingboard
                                                | CardType::Phasor
                                                | CardType::Saturn128K
                                                | CardType::RamWorksIII
                                            );
                                            if has_options {
                                                if ui.small_button("Options…").clicked() {
                                                    self.slot_options_open[slot] = true;
                                                }
                                            } else {
                                                ui.label(""); // spacer
                                            }
                                            ui.end_row();
                                        }
                                        // Aux slot row
                                        ui.label("Aux slot:");
                                        let aux_current = self.pending_config.aux_slot_card;
                                        egui::ComboBox::from_id_source("slot_aux")
                                            .selected_text(card_name(aux_current))
                                            .show_ui(ui, |ui| {
                                                for &card in &[
                                                    CardType::Empty,
                                                    CardType::Extended80Col,
                                                    CardType::Col80,
                                                    CardType::RamWorksIII,
                                                ] {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.aux_slot_card,
                                                        card,
                                                        card_name(card),
                                                    );
                                                }
                                            });
                                        ui.label(""); // spacer (no options button for aux)
                                        } // else (non-IIc)
                                        ui.end_row();
                                    });
                                ui.add_space(4.0);
                                ui.label(RichText::new(
                                    "Slot changes take effect after OK (requires reset)."
                                ).small());
                            }
                            6 => {
                                // Advanced tab
                                ui.add_space(4.0);
                                ui.label("Display:");
                                egui::Grid::new("adv_display_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Window scale:");
                                        ui.add(
                                            egui::Slider::new(
                                                &mut self.pending_config.window_scale,
                                                1..=4,
                                            )
                                            .suffix("×"),
                                        );
                                        ui.end_row();
                                    });
                                ui.checkbox(
                                    &mut self.pending_config.show_disk_status,
                                    "Show Disk II activity LEDs",
                                );
                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(4.0);
                                ui.label("Behaviour:");
                                ui.checkbox(
                                    &mut self.pending_config.confirm_reboot,
                                    "Confirm before reset",
                                );
                                ui.checkbox(
                                    &mut self.pending_config.scrolllock_toggle,
                                    "F10 key toggles pause (ScrollLock equivalent)",
                                );
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(4.0);
                                ui.label("Save state:");
                                ui.checkbox(
                                    &mut self.pending_config.save_state_on_exit,
                                    "Save state on exit / restore on launch",
                                );
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label("File:");
                                    let fname = self.pending_config.save_state_filename
                                        .as_deref()
                                        .unwrap_or("(auto)");
                                    ui.label(RichText::new(fname).monospace().small());
                                    if ui.small_button("Browse…").clicked()
                                        && let Some(path) = rfd::FileDialog::new()
                                            .set_title("Save State File")
                                            .add_filter("AWS YAML", &["yaml", "aws.yaml"])
                                            .save_file()
                                    {
                                        self.pending_config.save_state_filename =
                                            Some(path.to_string_lossy().into_owned());
                                    }
                                    if ui.small_button("Clear").clicked() {
                                        self.pending_config.save_state_filename = None;
                                    }
                                });
                            }
                            _ => {}
                        }

                        // ── Buttons ───────────────────────────────────────────
                        ui.add_space(8.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("  OK  ").clicked()     { apply_settings = true; }
                            if ui.button("Cancel").clicked() { cancel_settings = true; }
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
                // Resize window if scale changed, but only when not maximized.
                // In maximized state the OS controls the window size; we leave
                // it alone so the user can un-maximize and get the right size.
                if scale_changed && !frame.info().window_info.maximized {
                    let s = self.config.window_scale.max(1) as f32;
                    frame.set_window_size(egui::vec2(
                        SCREEN_W as f32 * s + BEVEL * 2.0 + BTN_PANEL_W + 24.0,
                        SCREEN_H as f32 * s + BEVEL * 2.0 + 80.0,
                    ));
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
