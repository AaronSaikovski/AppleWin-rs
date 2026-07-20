//! The bottom status bar (drive LEDs, disk name, hints).

use super::*;

impl EmulatorApp {
    pub(super) fn show_status_bar(&mut self, ctx: &egui::Context, in_logo_mode: bool) {
        // Snapshot disk state for use in closures (avoids borrow conflicts)
        let d1_name = Self::disk_display_name(&self.disk1).to_owned();
        let d1_loaded = self.disk1.is_some();
        let d1_activity = self.emu.bus.disk_drive_activity(self.disk_slot, 0);
        let d2_activity = self.emu.bus.disk_drive_activity(self.disk_slot, 1);

        // HDD activity: check all slots for a hard disk controller
        let hdd_activity = (0..apple2_core::card::NUM_SLOTS)
            .find(|&s| {
                self.emu
                    .bus
                    .cards
                    .slot(s)
                    .is_some_and(|c| c.card_type() == apple2_core::card::CardType::GenericHdd)
            })
            .map(|s| self.emu.bus.disk_drive_activity(s, 0))
            .unwrap_or_default();

        // ── Status bar (hidden when debugger is active) ──────────────────
        let debugger_fullscreen = self.show_debugger && self.debugger.active;
        if !debugger_fullscreen {
            egui::TopBottomPanel::bottom("statusbar")
                .frame(
                    egui::Frame::none()
                        .fill(WIN_FACE)
                        .stroke(Stroke::new(1.0_f32, WIN_SHADOW))
                        .inner_margin(egui::Margin::symmetric(6.0, 3.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if self.config.show_disk_status {
                            // Drive 1: LED + track
                            ui.label(RichText::new("1").small().monospace());
                            disk_led(ui, "", d1_activity.motor_on, d1_activity.writing);
                            ui.label(
                                RichText::new(format!("T{:02}", d1_activity.track))
                                    .small()
                                    .monospace(),
                            );
                            ui.add_space(4.0);

                            // Drive 2: LED + track
                            ui.label(RichText::new("2").small().monospace());
                            disk_led(ui, "", d2_activity.motor_on, d2_activity.writing);
                            ui.label(
                                RichText::new(format!("T{:02}", d2_activity.track))
                                    .small()
                                    .monospace(),
                            );
                            ui.add_space(4.0);

                            // HDD: LED
                            ui.label(RichText::new("H").small().monospace());
                            disk_led(ui, "", hdd_activity.motor_on, hdd_activity.writing);
                            ui.add_space(4.0);

                            // Disk name
                            ui.label(
                                RichText::new(if d1_loaded { d1_name.as_str() } else { "" })
                                    .small()
                                    .monospace(),
                            );
                        }
                        ui.separator();
                        if in_logo_mode {
                            ui.label(RichText::new("AppleWin-rs — Press any key to start").small());
                        } else {
                            // ui.label(
                            //     RichText::new(format!("PC:${pc:04X}")).small().monospace(),
                            // );
                            ui.add_space(6.0);
                            // ui.label(
                            //     RichText::new(format!("Cyc:{cycles}")).small().monospace(),
                            //);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new("F1:Reset  Ctrl+Esc:Quit").small());
                        });
                    });
                });
        } // end if !debugger_fullscreen (status bar)
    }
}
