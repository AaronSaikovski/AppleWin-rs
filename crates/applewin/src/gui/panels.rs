//! UI panels: menu bar, status bar, button strip, dialogs (reboot/about),
//! debugger panel, central screen panel, deferred-action application, and
//! drag-and-drop disk insertion.

use super::*;

/// UI actions requested by panel closures during a frame, applied after
/// all panels are laid out (so closures never need `&mut self` twice).
#[derive(Default)]
pub(super) struct DeferredActions {
    hard_reset: bool,
    reset: bool,
    quit: bool,
    load_disk1: bool,
    load_disk2: bool,
    eject_disk1: bool,
    eject_disk2: bool,
    swap: bool,
    fullscreen: bool,
    about: bool,
    show_settings: bool,
    screenshot: bool,
    load_hdd1: bool,
    load_hdd2: bool,
    eject_hdd1: bool,
    eject_hdd2: bool,
    recent_disk: Option<String>,
    recent_hdd: Option<String>,
}
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
                    .inner_margin(egui::style::Margin::symmetric(4.0, 2.0)),
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
                        .stroke(Stroke::new(1.0, WIN_SHADOW))
                        .inner_margin(egui::style::Margin::symmetric(6.0, 3.0)),
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
                        .inner_margin(egui::style::Margin::same(5.0)),
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

    pub(super) fn show_debugger_panel(&mut self, ctx: &egui::Context) {
        // ── Debugger (renders into framebuffer, command bar at bottom) ────
        if self.show_debugger && self.debugger.active {
            use apple2_debugger::commands::{self, CmdResult, CpuRegs};
            use apple2_debugger::disasm::{disassemble_one, format_instruction};

            let paused = self.debugger.active;
            let mut do_step = false;
            let mut do_step_over = false;
            let mut do_step_out = false;
            let mut do_resume = false;
            let mut do_exec_cmd = false;
            let cmd_empty = self.debugger_cmd_input.is_empty();

            // AppleWin-compatible keyboard shortcuts
            ctx.input(|i| {
                for event in &i.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } = event
                    {
                        let ctrl = modifiers.ctrl || modifiers.command;
                        let shift = modifiers.shift;
                        match key {
                            Key::Space if cmd_empty && !ctrl && !shift => do_step = true,
                            Key::Space if cmd_empty && ctrl && !shift => do_step_over = true,
                            Key::Space if cmd_empty && shift && !ctrl => do_step_out = true,
                            Key::F5 => do_resume = true,
                            Key::ArrowUp if cmd_empty && !ctrl && !shift => {
                                self.debugger.goto_addr = Some(
                                    self.debugger
                                        .goto_addr
                                        .unwrap_or(self.emu.cpu.pc)
                                        .wrapping_sub(1),
                                );
                            }
                            Key::ArrowDown if cmd_empty && !ctrl && !shift => {
                                let start = self.debugger.goto_addr.unwrap_or(self.emu.cpu.pc);
                                let instr = apple2_debugger::disasm::disassemble_one(start, |a| {
                                    self.emu.bus.read_raw(a)
                                });
                                self.debugger.goto_addr =
                                    Some(start.wrapping_add(instr.bytes as u16));
                            }
                            Key::PageUp if !ctrl && !shift => {
                                self.debugger.goto_addr = Some(
                                    self.debugger
                                        .goto_addr
                                        .unwrap_or(self.emu.cpu.pc)
                                        .wrapping_sub(0x20),
                                );
                            }
                            Key::PageDown if !ctrl && !shift => {
                                self.debugger.goto_addr = Some(
                                    self.debugger
                                        .goto_addr
                                        .unwrap_or(self.emu.cpu.pc)
                                        .wrapping_add(0x20),
                                );
                            }
                            Key::PageUp if shift => {
                                self.debugger.goto_addr = Some(
                                    self.debugger
                                        .goto_addr
                                        .unwrap_or(self.emu.cpu.pc)
                                        .wrapping_sub(0x100),
                                );
                            }
                            Key::PageDown if shift => {
                                self.debugger.goto_addr = Some(
                                    self.debugger
                                        .goto_addr
                                        .unwrap_or(self.emu.cpu.pc)
                                        .wrapping_add(0x100),
                                );
                            }
                            Key::Home => {
                                self.debugger.goto_addr = None;
                            }
                            Key::ArrowRight if ctrl && cmd_empty => {
                                if let Some(addr) = self.debugger.goto_addr {
                                    self.emu.cpu.pc = addr;
                                    self.debugger.print(format!("PC set to ${addr:04X}"));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            });

            // Command input bar
            let cmd_id = egui::Id::new("dbg_cmd_input");
            egui::TopBottomPanel::bottom("dbg_cmd")
                .frame(
                    egui::Frame::none()
                        .fill(Color32::from_rgb(16, 16, 32))
                        .inner_margin(egui::style::Margin::symmetric(6.0, 2.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Spc:Step C-Spc:Over S-Spc:Out F5:Go F7:Exit")
                                .monospace()
                                .small()
                                .color(Color32::from_rgb(100, 100, 100)),
                        );
                        ui.label(
                            RichText::new(">")
                                .monospace()
                                .color(Color32::from_rgb(255, 128, 0)),
                        );
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.debugger_cmd_input)
                                .id(cmd_id)
                                .desired_width(ui.available_width() - 8.0)
                                .font(FontId::monospace(12.0))
                                .text_color(Color32::WHITE),
                        );
                        if !resp.has_focus() {
                            resp.request_focus();
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            do_exec_cmd = true;
                        }
                    });
                });
            if do_exec_cmd {
                ctx.memory_mut(|m| m.request_focus(cmd_id));
            }

            // Apply deferred actions
            if do_resume {
                self.debugger.deactivate();
                self.show_debugger = false;
            }
            if do_step && paused {
                if self.debugger.trace.enabled {
                    use apple2_debugger::trace::TraceEntry;
                    let instr = disassemble_one(self.emu.cpu.pc, |a| self.emu.bus.read_raw(a));
                    self.debugger.trace.push(TraceEntry {
                        pc: self.emu.cpu.pc,
                        opcode: instr.opcode,
                        a: self.emu.cpu.a,
                        x: self.emu.cpu.x,
                        y: self.emu.cpu.y,
                        sp: self.emu.cpu.sp,
                        flags: self.emu.cpu.flags.bits(),
                        cycles: self.emu.cpu.cycles,
                        text: format_instruction(&instr),
                    });
                }
                self.emu.step();
                self.debugger.stop_reason = apple2_debugger::state::StopReason::Step;
            }
            if do_step_over && paused {
                let opcode = self.emu.bus.read_raw(self.emu.cpu.pc);
                if opcode == 0x20 {
                    self.debugger.step_over_target = Some(self.emu.cpu.pc.wrapping_add(3));
                    self.debugger.deactivate();
                } else {
                    self.emu.step();
                    self.debugger.stop_reason = apple2_debugger::state::StopReason::StepOver;
                }
            }
            if do_step_out && paused {
                self.debugger.step_out_sp = Some(self.emu.cpu.sp);
                self.debugger.deactivate();
            }

            // Execute console command
            if do_exec_cmd {
                let cmd_text = self.debugger_cmd_input.clone();
                self.debugger_cmd_input.clear();
                self.debugger.print(format!("> {cmd_text}"));
                let regs = CpuRegs {
                    a: self.emu.cpu.a,
                    x: self.emu.cpu.x,
                    y: self.emu.cpu.y,
                    sp: self.emu.cpu.sp,
                    pc: self.emu.cpu.pc,
                    flags: self.emu.cpu.flags.bits(),
                    cycles: self.emu.cpu.cycles,
                };
                let result = commands::execute_command(
                    &mut self.debugger,
                    &cmd_text,
                    self.emu.cpu.pc,
                    regs,
                    |a| self.emu.bus.read_raw(a),
                );
                match result {
                    CmdResult::Output(lines) => {
                        self.debugger.print_lines(&lines);
                    }
                    CmdResult::Go => {
                        self.debugger.deactivate();
                        self.show_debugger = false;
                    }
                    CmdResult::Step => {
                        self.emu.step();
                        self.debugger.stop_reason = apple2_debugger::state::StopReason::Step;
                    }
                    CmdResult::StepOver => {
                        let opcode = self.emu.bus.read_raw(self.emu.cpu.pc);
                        if opcode == 0x20 {
                            self.debugger.step_over_target = Some(self.emu.cpu.pc.wrapping_add(3));
                            self.debugger.deactivate();
                        } else {
                            self.emu.step();
                        }
                    }
                    CmdResult::StepOut => {
                        self.debugger.step_out_sp = Some(self.emu.cpu.sp);
                        self.debugger.deactivate();
                    }
                    CmdResult::Trace(n) => {
                        self.debugger.trace.enabled = true;
                        self.debugger.trace_remaining = n;
                    }
                    CmdResult::SetPC(addr) => {
                        self.emu.cpu.pc = addr;
                        self.debugger.deactivate();
                    }
                    CmdResult::MemWrite(addr, val) => {
                        self.emu.bus.write_raw(addr, val);
                        self.debugger.print(format!("  ${addr:04X} = {val:02X}"));
                    }
                    CmdResult::SetReg(reg, val) => match reg {
                        'A' => self.emu.cpu.a = val as u8,
                        'X' => self.emu.cpu.x = val as u8,
                        'Y' => self.emu.cpu.y = val as u8,
                        'S' | 'P' if val > 0xFF => self.emu.cpu.pc = val,
                        'S' => self.emu.cpu.sp = val as u8,
                        'P' => self.emu.cpu.pc = val,
                        _ => {}
                    },
                    CmdResult::Nop => {}
                    CmdResult::Error(msg) => {
                        self.debugger.print(format!("Error: {msg}"));
                    }
                    CmdResult::ToggleBreak => {
                        if self.debugger.active {
                            self.debugger.deactivate();
                        } else {
                            self.debugger
                                .activate(apple2_debugger::state::StopReason::UserBreak);
                        }
                    }
                }
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

    pub(super) fn show_central_panel(
        &mut self,
        ctx: &egui::Context,
        in_logo_mode: bool,
        tex_id: Option<egui::TextureId>,
    ) {
        let debugger_fullscreen = self.show_debugger && self.debugger.active;

        // ── Central panel — Apple II screen / debugger display ─────────────
        let central_bg = if debugger_fullscreen {
            Color32::BLACK
        } else {
            WIN_FACE
        };
        let central_margin = if debugger_fullscreen { 0.0 } else { 8.0 };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(central_bg)
                    .inner_margin(egui::style::Margin::same(central_margin)),
            )
            .show(ctx, |ui| {
                let avail = ui.available_rect_before_wrap();
                let sw = SCREEN_W as f32;
                let sh = SCREEN_H as f32;
                if debugger_fullscreen {
                    // Debugger mode: fill area, no bevel — fractional scale + linear filter
                    let scale_w = avail.width() / sw;
                    let scale_h = avail.height() / sh;
                    let scale = scale_w.min(scale_h).max(1.0);
                    let disp_w = sw * scale;
                    let disp_h = sh * scale;
                    let ox = avail.left() + ((avail.width() - disp_w) / 2.0).max(0.0);
                    let oy = avail.top() + ((avail.height() - disp_h) / 2.0).max(0.0);
                    let screen = Rect::from_min_size(Pos2::new(ox, oy), Vec2::new(disp_w, disp_h));
                    if let Some(tid) = tex_id {
                        ui.painter().image(
                            tid,
                            screen,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    ui.allocate_rect(avail, Sense::hover());
                } else {
                    // Normal mode: bevel + centred screen — fractional scale + linear filter
                    let avail_w = avail.width() - BEVEL * 2.0;
                    let avail_h = avail.height() - BEVEL * 2.0;
                    let scale_w = avail_w / sw;
                    let scale_h = avail_h / sh;
                    let scale = scale_w.min(scale_h).max(1.0);
                    let outer_w = sw * scale + BEVEL * 2.0;
                    let outer_h = sh * scale + BEVEL * 2.0;

                    let ox = avail.left() + ((avail.width() - outer_w) / 2.0).max(0.0);
                    let oy = avail.top() + ((avail.height() - outer_h) / 2.0).max(0.0);

                    let outer = Rect::from_min_size(Pos2::new(ox, oy), Vec2::new(outer_w, outer_h));
                    let screen = outer.shrink(BEVEL);

                    let painter = ui.painter();
                    draw_sunken_bevel(painter, outer);

                    if let Some(tid) = tex_id {
                        painter.image(
                            tid,
                            screen,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }

                    // ── Logo mode overlay — version + prompt ──────────────────
                    if in_logo_mode {
                        let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
                        // Version string position mirrors the C++ DRAWVERSION macro:
                        // scale*540 x scale*358 relative to the 560×384 logo area.
                        let vx = screen.left() + screen.width() * (540.0 / 560.0);
                        let vy = screen.top() + screen.height() * (358.0 / 384.0);
                        let vfont = FontId::proportional(13.0);
                        // 3-layer rendering matching C++ hi-colour path:
                        // (+1,+1) dark shadow, (-1,-1) light highlight, (0,0) main purple
                        painter.text(
                            Pos2::new(vx + 1.0, vy + 1.0),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont.clone(),
                            Color32::from_rgb(0x30, 0x30, 0x70),
                        );
                        painter.text(
                            Pos2::new(vx - 1.0, vy - 1.0),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont.clone(),
                            Color32::from_rgb(0xC0, 0x70, 0xE0),
                        );
                        painter.text(
                            Pos2::new(vx, vy),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont,
                            Color32::from_rgb(0x70, 0x30, 0xE0),
                        );
                        // "Press any key" prompt at bottom-centre
                        let px = screen.center().x;
                        let py = screen.bottom() - 10.0;
                        let pfont = FontId::proportional(13.0);
                        painter.text(
                            Pos2::new(px + 1.0, py + 1.0),
                            egui::Align2::CENTER_BOTTOM,
                            "Press any key to start",
                            pfont.clone(),
                            Color32::BLACK,
                        );
                        painter.text(
                            Pos2::new(px, py),
                            egui::Align2::CENTER_BOTTOM,
                            "Press any key to start",
                            pfont,
                            Color32::WHITE,
                        );
                    }

                    ui.allocate_rect(outer, Sense::hover());
                } // end else (normal mode)
            });
    }

    pub(super) fn apply_deferred_actions(
        &mut self,
        frame: &mut eframe::Frame,
        act: DeferredActions,
        in_logo_mode: bool,
    ) {
        // ── Apply deferred actions ────────────────────────────────────────
        if act.hard_reset {
            if self.config.confirm_reboot {
                self.pending_reset = Some(true);
            } else {
                self.reset(true);
            }
        }
        if act.reset {
            if self.config.confirm_reboot {
                self.pending_reset = Some(false);
            } else {
                self.reset(false);
            }
        }
        if act.quit {
            self.config.save();
            frame.close();
        }
        if act.about {
            self.show_about = true;
        }
        if act.show_settings {
            self.pending_config = self.config.clone();
            self.show_settings = true;
        }
        if act.fullscreen {
            self.fullscreen = !self.fullscreen;
            frame.set_fullscreen(self.fullscreen);
        }
        if act.swap {
            // Swap path names
            std::mem::swap(&mut self.disk1, &mut self.disk2);
            std::mem::swap(&mut self.config.last_disk1, &mut self.config.last_disk2);
            // Re-load both drives so the card reflects the swap
            let d1 = self.disk1.clone();
            let d2 = self.disk2.clone();
            Self::reload_disk(&mut self.emu, self.disk_slot, 0, &d1);
            Self::reload_disk(&mut self.emu, self.disk_slot, 1, &d2);
        }
        if act.eject_disk1 {
            self.emu.bus.eject_disk(self.disk_slot, 0);
            self.disk1 = None;
            self.config.last_disk1 = None;
            self.config.save();
        }
        if act.eject_disk2 {
            self.emu.bus.eject_disk(self.disk_slot, 1);
            self.disk2 = None;
            self.config.last_disk2 = None;
            self.config.save();
        }
        if act.load_disk1 {
            let start_dir = self.config.last_disk_dir.as_deref();
            if let Some(path) = open_disk_dialog("Load Disk 1", start_dir) {
                let loaded = if let Some(ref mut iigs) = self.iigs {
                    Self::load_iigs_disk(iigs, 0, &path)
                } else if let Ok(data) = std::fs::read(&path) {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    self.emu.bus.load_disk(self.disk_slot, 0, &data, &ext);
                    true
                } else {
                    false
                };
                if loaded {
                    let path_str = path.to_string_lossy().into_owned();
                    self.config.add_recent_disk(&path_str);
                    self.config.last_disk1 = Some(path_str);
                    self.config.last_disk_dir =
                        path.parent().map(|p| p.to_string_lossy().into_owned());
                    self.disk1 = Some(path);
                    self.config.save();
                }
            }
        }
        if act.load_disk2 {
            let start_dir = self.config.last_disk_dir.as_deref();
            if let Some(path) = open_disk_dialog("Load Disk 2", start_dir) {
                let loaded = if let Some(ref mut iigs) = self.iigs {
                    Self::load_iigs_disk(iigs, 1, &path)
                } else if let Ok(data) = std::fs::read(&path) {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    self.emu.bus.load_disk(self.disk_slot, 1, &data, &ext);
                    true
                } else {
                    false
                };
                if loaded {
                    let path_str = path.to_string_lossy().into_owned();
                    self.config.add_recent_disk(&path_str);
                    self.config.last_disk2 = Some(path_str);
                    self.config.last_disk_dir =
                        path.parent().map(|p| p.to_string_lossy().into_owned());
                    self.disk2 = Some(path);
                    self.config.save();
                }
            }
        }
        // Load disk from recent list into drive 1
        if let Some(path_str) = act.recent_disk {
            let path = PathBuf::from(&path_str);
            let loaded = if let Some(ref mut iigs) = self.iigs {
                Self::load_iigs_disk(iigs, 0, &path)
            } else if let Ok(data) = std::fs::read(&path) {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                self.emu.bus.load_disk(self.disk_slot, 0, &data, &ext);
                self.emu.bus.set_disk_path(self.disk_slot, 0, path.clone());
                true
            } else {
                false
            };
            if loaded {
                self.config.add_recent_disk(&path_str);
                self.config.last_disk1 = Some(path_str);
                self.config.last_disk_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
                self.disk1 = Some(path);
                self.config.save();
            }
        }
        // HDD: load / eject
        if act.load_hdd1 {
            let start_dir = self.config.last_hdd_dir.as_deref();
            if let Some(path) = open_hdd_dialog("Load HDD 1", start_dir)
                && let Ok(data) = std::fs::read(&path)
            {
                let path_str = path.to_string_lossy().into_owned();
                self.config.add_recent_hdd(&path_str);
                self.config.last_hdd1 = Some(path_str);
                self.config.last_hdd_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
                // Apply to any installed HD card
                for slot in 0..apple2_core::card::NUM_SLOTS {
                    if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                        && card.card_type() == apple2_core::card::CardType::GenericHdd
                    {
                        if let Some(hd) = card
                            .as_any_mut()
                            .downcast_mut::<apple2_core::cards::hd::HdCard>()
                        {
                            hd.load_image(0, data);
                        }
                        break;
                    }
                }
                self.config.save();
            }
        }
        if act.load_hdd2 {
            let start_dir = self.config.last_hdd_dir.as_deref();
            if let Some(path) = open_hdd_dialog("Load HDD 2", start_dir)
                && let Ok(data) = std::fs::read(&path)
            {
                let path_str = path.to_string_lossy().into_owned();
                self.config.add_recent_hdd(&path_str);
                self.config.last_hdd2 = Some(path_str);
                self.config.last_hdd_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
                for slot in 0..apple2_core::card::NUM_SLOTS {
                    if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                        && card.card_type() == apple2_core::card::CardType::GenericHdd
                    {
                        if let Some(hd) = card
                            .as_any_mut()
                            .downcast_mut::<apple2_core::cards::hd::HdCard>()
                        {
                            hd.load_image(1, data);
                        }
                        break;
                    }
                }
                self.config.save();
            }
        }
        if act.eject_hdd1 {
            self.config.last_hdd1 = None;
            self.config.save();
        }
        if act.eject_hdd2 {
            self.config.last_hdd2 = None;
            self.config.save();
        }
        if let Some(path_str) = act.recent_hdd {
            let path = PathBuf::from(&path_str);
            if let Ok(data) = std::fs::read(&path) {
                self.config.add_recent_hdd(&path_str);
                self.config.last_hdd1 = Some(path_str);
                self.config.last_hdd_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
                for slot in 0..apple2_core::card::NUM_SLOTS {
                    if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                        && card.card_type() == apple2_core::card::CardType::GenericHdd
                    {
                        if let Some(hd) = card
                            .as_any_mut()
                            .downcast_mut::<apple2_core::cards::hd::HdCard>()
                        {
                            hd.load_image(0, data);
                        }
                        break;
                    }
                }
                self.config.save();
            }
        }

        // Screenshot from menu or F12 key
        if act.screenshot && !in_logo_mode {
            self.render_apple2();
            save_screenshot(self.fb.pixels_as_bytes(), SCREEN_W, SCREEN_H);
        }
    }

    pub(super) fn handle_drag_and_drop(&mut self, ctx: &egui::Context) {
        // ── Drag-and-drop disk insertion ─────────────────────────────
        // First dropped file → drive 1, second → drive 2.
        {
            let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
            let disk_exts = [
                "dsk", "do", "po", "nib", "nb2", "woz", "d13", "gz", "zip", "2mg", "2img",
            ];
            for (i, file) in dropped.iter().enumerate() {
                if let Some(ref path) = file.path {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if disk_exts.iter().any(|&e| e == ext) {
                        let drive = i.min(1); // 0 or 1
                        let loaded = if let Some(ref mut iigs) = self.iigs {
                            Self::load_iigs_disk(iigs, drive, path)
                        } else if let Ok(data) = std::fs::read(path) {
                            self.emu.bus.load_disk(self.disk_slot, drive, &data, &ext);
                            self.emu
                                .bus
                                .set_disk_path(self.disk_slot, drive, path.clone());
                            true
                        } else {
                            false
                        };
                        if loaded {
                            let path_str = path.to_string_lossy().into_owned();
                            self.config.add_recent_disk(&path_str);
                            if drive == 0 {
                                self.disk1 = Some(path.clone());
                                self.config.last_disk1 = Some(path_str);
                            } else {
                                self.disk2 = Some(path.clone());
                                self.config.last_disk2 = Some(path_str);
                            }
                            self.config.last_disk_dir =
                                path.parent().map(|p| p.to_string_lossy().into_owned());
                            self.config.save();
                        }
                    }
                }
            }
        }
    }
}
// ── File dialog ───────────────────────────────────────────────────────────

fn open_disk_dialog(title: &str, start_dir: Option<&str>) -> Option<PathBuf> {
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

fn open_hdd_dialog(title: &str, start_dir: Option<&str>) -> Option<PathBuf> {
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
