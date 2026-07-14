//! The right-hand icon button strip (help, reset, disks, swap, fullscreen,
//! debugger, settings).

use super::*;

impl EmulatorApp {
    pub(super) fn show_button_strip(&mut self, ctx: &egui::Context, act: &mut DeferredActions) {
        let debugger_fullscreen = self.show_debugger && self.debugger.active;

        // ── Right button strip (hidden when debugger is active) ───────────
        let icons = self.icons.as_ref();
        if !debugger_fullscreen {
            egui::SidePanel::right("buttons")
                .exact_width(BTN_PANEL_W)
                .resizable(false)
                .frame(
                    egui::Frame::none()
                        .fill(WIN_FACE)
                        .stroke(Stroke::new(1.0, WIN_SHADOW))
                        .inner_margin(egui::Margin::same(5.0)),
                )
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        let sz = Vec2::new(43.0, 41.0);
                        let isz = Vec2::new(41.0, 41.0);
                        ui.add_space(2.0);
                        if icon_btn(
                            ui,
                            icons.and_then(|ic| ic.help.as_ref()),
                            "?",
                            "Help / About",
                            sz,
                            isz,
                        )
                        .clicked()
                        {
                            act.about = true;
                        }
                        ui.add_space(2.0);
                        // Matches AppleWin BTN_RUN logic:
                        //   Ctrl+click → CtrlReset (soft reset, warm CPU)
                        //   click      → ResetMachineState (power cycle)
                        if icon_btn(
                            ui,
                            icons.and_then(|ic| ic.run.as_ref()),
                            "↺",
                            "Reset  (Ctrl+click = soft reset, click = power cycle)",
                            sz,
                            isz,
                        )
                        .clicked()
                        {
                            let ctrl = ui.ctx().input(|i| i.modifiers.ctrl || i.modifiers.command);
                            if ctrl {
                                act.reset = true;
                            } else {
                                act.hard_reset = true;
                            }
                        }
                        ui.add_space(2.0);
                        if icon_btn(
                            ui,
                            icons.and_then(|ic| ic.d1.as_ref()),
                            "①",
                            "Load Disk 1",
                            sz,
                            isz,
                        )
                        .clicked()
                        {
                            act.load_disk1 = true;
                        }
                        ui.add_space(2.0);
                        if icon_btn(
                            ui,
                            icons.and_then(|ic| ic.d2.as_ref()),
                            "②",
                            "Load Disk 2",
                            sz,
                            isz,
                        )
                        .clicked()
                        {
                            act.load_disk2 = true;
                        }
                        ui.add_space(2.0);
                        if icon_btn(
                            ui,
                            icons.and_then(|ic| ic.swap.as_ref()),
                            "⇄",
                            "Swap Drives",
                            sz,
                            isz,
                        )
                        .clicked()
                        {
                            act.swap = true;
                        }
                        ui.add_space(2.0);
                        if icon_btn(
                            ui,
                            icons.and_then(|ic| ic.full.as_ref()),
                            "⛶",
                            "Fullscreen (F11)",
                            sz,
                            isz,
                        )
                        .clicked()
                        {
                            act.fullscreen = true;
                        }
                        ui.add_space(2.0);
                        if icon_btn(
                            ui,
                            icons.and_then(|ic| ic.debug.as_ref()),
                            "⚙",
                            "Debugger",
                            sz,
                            isz,
                        )
                        .clicked()
                        {
                            if self.show_debugger && self.debugger.active {
                                self.show_debugger = false;
                                self.debugger.deactivate();
                            } else {
                                self.show_debugger = true;
                                self.debugger
                                    .activate(apple2_debugger::state::StopReason::UserBreak);
                            }
                        }
                        ui.add_space(2.0);
                        if icon_btn(
                            ui,
                            icons.and_then(|ic| ic.setup.as_ref()),
                            "⚙",
                            "Settings",
                            sz,
                            isz,
                        )
                        .clicked()
                        {
                            act.show_settings = true;
                        }
                    });
                });
        } // end if !debugger_fullscreen (button strip)
    }
}
