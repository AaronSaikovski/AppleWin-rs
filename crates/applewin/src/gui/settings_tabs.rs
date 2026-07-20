//! The individual tabs of the Settings dialog. Each renders into the `ui`
//! supplied by [`EmulatorApp::show_settings_dialog`] and edits
//! `self.pending_config` (applied only when the user clicks OK).

use super::*;

impl EmulatorApp {
    pub(super) fn render_machine_tab(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("machine_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Computer type:");
                egui::ComboBox::from_id_salt("machine_type")
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
                        egui::ComboBox::from_id_salt("cpu_type")
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
                    egui::ComboBox::from_id_salt("iigs_ram")
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
                    let rom_label = self
                        .pending_config
                        .iigs_rom_path
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

    pub(super) fn render_video_tab(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("video_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Video type:");
                egui::ComboBox::from_id_salt("video_type")
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
                        ((c >> 8) & 0xFF) as u8,
                        (c & 0xFF) as u8,
                    ];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        self.pending_config.monochrome_color =
                            ((rgb[0] as u32) << 16) | ((rgb[1] as u32) << 8) | (rgb[2] as u32);
                    }
                    ui.end_row();
                }
                ui.label("Refresh rate:");
                ui.horizontal(|ui| {
                    ui.radio_value(
                        &mut self.pending_config.video_refresh_hz,
                        60,
                        "60 Hz (NTSC)",
                    );
                    ui.radio_value(&mut self.pending_config.video_refresh_hz, 50, "50 Hz (PAL)");
                });
                ui.end_row();
            });
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.pending_config.scanlines,
            "CRT scanlines (half-scanline darkening)",
        );
        ui.checkbox(
            &mut self.pending_config.color_vertical_blend,
            "Colour vertical blend",
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Cycles per frame: {}",
                self.pending_config.cycles_per_frame()
            ))
            .small(),
        );
    }

    pub(super) fn render_audio_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.add(
            egui::Slider::new(&mut self.pending_config.master_volume, 0..=100)
                .text("Master volume")
                .suffix("%"),
        );
    }

    pub(super) fn render_speed_tab(&mut self, ui: &mut egui::Ui) {
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
                && ui.small_button("Reset to normal").clicked()
            {
                self.pending_config.emulation_speed = SPEED_NORMAL;
            }
        });
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.pending_config.enhanced_disk_speed,
            "Enhanced disk speed  (16× while motor is spinning)",
        );
        ui.add_space(4.0);
        ui.label(RichText::new("Speeds up disk-based game boot times significantly.").small());
    }

    pub(super) fn render_input_tab(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("input_grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                ui.label("Joystick 1:");
                egui::ComboBox::from_id_salt("joy0_type")
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
                egui::ComboBox::from_id_salt("joy1_type")
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
        ui.checkbox(
            &mut self.pending_config.joystick_swap_buttons,
            "Swap joystick buttons",
        );
        ui.checkbox(
            &mut self.pending_config.joystick_autofire,
            "Auto-fire button 0",
        );
        ui.checkbox(
            &mut self.pending_config.joystick_self_centering,
            "Self-centring joystick",
        );
        ui.checkbox(
            &mut self.pending_config.joystick_cursor_control,
            "Cursor keys control joystick",
        );
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.checkbox(
            &mut self.pending_config.mouse_crosshair,
            "Show crosshair mouse cursor",
        );
        ui.checkbox(
            &mut self.pending_config.mouse_restrict_to_window,
            "Restrict mouse to window",
        );
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
        ui.label(
            RichText::new("Physical gamepads, keyboard (Arrow/NumPad), and mouse modes supported.")
                .small(),
        );
    }

    pub(super) fn render_slots_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        egui::Grid::new("slots_grid")
            .num_columns(3)
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                if self.pending_config.machine_type.is_iic() {
                    // Apple IIc: fixed built-in peripherals (read-only)
                    let iic_slots = [
                        "(empty)",              // slot 0
                        "Serial (modem)",       // slot 1
                        "Serial (printer)",     // slot 2
                        "80-column (built-in)", // slot 3
                        "Mouse",                // slot 4
                        "(empty)",              // slot 5
                        "Disk II",              // slot 6
                        "(empty)",              // slot 7
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
                        egui::ComboBox::from_id_salt(format!("slot_{slot}"))
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
                    egui::ComboBox::from_id_salt("slot_aux")
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
        ui.label(RichText::new("Slot changes take effect after OK (requires reset).").small());
    }

    pub(super) fn render_advanced_tab(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label("Display:");
        egui::Grid::new("adv_display_grid")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("Window scale:");
                ui.add(egui::Slider::new(&mut self.pending_config.window_scale, 1..=4).suffix("×"));
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
            let fname = self
                .pending_config
                .save_state_filename
                .as_deref()
                .unwrap_or("(auto)");
            ui.label(RichText::new(fname).monospace().small());
            if ui.small_button("Browse…").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Save State File")
                    .add_filter("AWS YAML", &["yaml", "aws.yaml"])
                    .save_file()
            {
                self.pending_config.save_state_filename = Some(path.to_string_lossy().into_owned());
            }
            if ui.small_button("Clear").clicked() {
                self.pending_config.save_state_filename = None;
            }
        });
    }
}
