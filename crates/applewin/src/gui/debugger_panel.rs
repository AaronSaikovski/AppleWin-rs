//! The debugger command bar and its keyboard shortcuts / command dispatch.

use super::*;

impl EmulatorApp {
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
                        .inner_margin(egui::Margin::symmetric(6.0, 2.0)),
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
}
