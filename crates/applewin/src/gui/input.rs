//! Input handling: keyboard events and shortcuts, clipboard paste,
//! joystick/paddle emulation (gamepad, keypad, and mouse modes).

use super::*;

impl EmulatorApp {
    /// Keyboard, paste, joystick/gamepad, and key-triggered actions.
    /// Returns `true` when the user requested quit (Ctrl+Esc).
    pub(super) fn handle_input(&mut self, ctx: &egui::Context, in_logo_mode: bool) -> bool {
        // ── Collect input events ──────────────────────────────────────────
        let mut key_queue: Vec<u8> = Vec::new();
        let mut do_reset: bool = false;
        let mut do_hard_reset: bool = false;
        let mut do_quit: bool = false;
        let mut any_key: bool = false;
        let mut paste_text: Option<String> = None;
        let mut take_screenshot: bool = false;
        let mut video_shortcut: Option<VideoType> = None;
        let mut do_save_state: bool = false;
        let mut do_load_state: bool = false;
        let mut speed_shortcut: Option<u32> = None;
        let mut toggle_wav_rec: bool = false;
        let mut alt_left: bool = false;
        let mut alt_right: bool = false;

        // Only process Event::Key with repeat:false — this fires exactly once
        // per physical key-down, never for OS auto-repeat.  Event::Text is
        // intentionally ignored because it fires on every repeat frame,
        // causing the Apple II to see the same letter dozens of times/second.
        ctx.input(|i| {
            for event in &i.events {
                // Paste event — filled by Ctrl+V or OS paste gesture.
                if let egui::Event::Paste(text) = event {
                    paste_text = Some(text.clone());
                    continue;
                }
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
                    let dbg_on = self.show_debugger && self.debugger.active;
                    match key {
                        // ── Global shortcuts (always active) ─────────
                        Key::F1 => do_hard_reset = true,
                        // F7 — toggle debugger (original AppleWin key)
                        Key::F7 if !ctrl && !shift => {
                            if self.show_debugger && self.debugger.active {
                                self.show_debugger = false;
                                self.debugger.deactivate();
                            } else {
                                self.show_debugger = true;
                                self.debugger
                                    .activate(apple2_debugger::state::StopReason::UserBreak);
                            }
                        }
                        // F10 — also toggles debugger (compat)
                        Key::F10 if self.config.scrolllock_toggle => {
                            if self.show_debugger && self.debugger.active {
                                self.show_debugger = false;
                                self.debugger.deactivate();
                            } else {
                                self.show_debugger = true;
                                self.debugger
                                    .activate(apple2_debugger::state::StopReason::UserBreak);
                            }
                        }
                        // Save / load state
                        Key::F11 if shift => do_load_state = true,
                        Key::F11 if !shift => do_save_state = true,
                        Key::F9 => toggle_wav_rec = true,
                        Key::F12 => take_screenshot = true,
                        Key::Escape if ctrl => do_quit = true,
                        Key::F2 if ctrl => do_reset = true,
                        // Speed control
                        Key::Num0 if ctrl => {
                            speed_shortcut = Some(40);
                        }
                        Key::Num1 if ctrl => {
                            speed_shortcut = Some(10);
                        }
                        Key::Num3 if ctrl => {
                            speed_shortcut = Some(30);
                        }
                        // Video mode shortcuts
                        Key::Num4 if ctrl => {
                            video_shortcut = Some(VideoType::MonoWhite);
                        }
                        Key::Num5 if ctrl => {
                            video_shortcut = Some(VideoType::MonoGreen);
                        }
                        Key::Num6 if ctrl => {
                            video_shortcut = Some(VideoType::ColorTV);
                        }
                        Key::Num7 if ctrl => {
                            video_shortcut = Some(VideoType::ColorIdealized);
                        }
                        Key::Num8 if ctrl => {
                            video_shortcut = Some(VideoType::ColorRGB);
                        }
                        Key::Num9 if ctrl => {
                            video_shortcut = Some(VideoType::ColorMonitorNtsc);
                        }
                        // ── Swallow keys when debugger is active ─────
                        _ if dbg_on => {}
                        // ── Apple II keys (only when debugger is NOT active) ─
                        Key::Enter => {
                            any_key = true;
                            key_queue.push(0x0D);
                        }
                        Key::Backspace => {
                            any_key = true;
                            key_queue.push(0x7F);
                        }
                        Key::Escape => {
                            any_key = true;
                            key_queue.push(0x1B);
                        }
                        Key::Tab => {
                            any_key = true;
                            key_queue.push(0x09);
                        }
                        Key::ArrowLeft => {
                            any_key = true;
                            key_queue.push(0x08);
                        }
                        Key::ArrowRight => {
                            any_key = true;
                            key_queue.push(0x15);
                        }
                        Key::ArrowUp => {
                            any_key = true;
                            key_queue.push(0x0B);
                        }
                        Key::ArrowDown => {
                            any_key = true;
                            key_queue.push(0x0A);
                        }
                        key if ctrl => {
                            let c: Option<u8> = match key {
                                Key::A => Some(0x01),
                                Key::B => Some(0x02),
                                Key::C => Some(0x03),
                                Key::D => Some(0x04),
                                Key::E => Some(0x05),
                                Key::F => Some(0x06),
                                Key::G => Some(0x07),
                                Key::H => Some(0x08),
                                Key::I => Some(0x09),
                                Key::J => Some(0x0A),
                                Key::K => Some(0x0B),
                                Key::L => Some(0x0C),
                                Key::M => Some(0x0D),
                                Key::N => Some(0x0E),
                                Key::O => Some(0x0F),
                                Key::P => Some(0x10),
                                Key::Q => Some(0x11),
                                Key::R => Some(0x12),
                                Key::S => Some(0x13),
                                Key::T => Some(0x14),
                                Key::U => Some(0x15),
                                Key::V => None,
                                Key::W => Some(0x17),
                                Key::X => Some(0x18),
                                Key::Y => Some(0x19),
                                Key::Z => Some(0x1A),
                                _ => None,
                            };
                            if let Some(c) = c {
                                any_key = true;
                                key_queue.push(c);
                            }
                        }
                        key => {
                            any_key = true;
                            if let Some(c) = apple2_ascii_for_key(*key, shift) {
                                key_queue.push(c);
                            }
                        }
                    }
                }
            }
        });

        // Any key press exits logo mode and starts the emulator
        if in_logo_mode && any_key {
            self.reset(true);
        }

        // Apply video mode shortcut (Ctrl+1..5)
        if let Some(vt) = video_shortcut {
            self.config.video_type = vt;
            self.renderer.tv_mode = matches!(
                vt,
                crate::config::VideoType::ColorTV | crate::config::VideoType::MonoTV
            );
            self.renderer.mono_tint = self.config.mono_tint();
            self.config.save();
        }

        // Process clipboard paste text → fill paste_buf.
        // Converts to Apple II ASCII: uppercase letters, CR newlines.
        if let Some(text) = paste_text {
            for ch in text.chars() {
                let b: u8 = match ch {
                    '\n' | '\r' => 0x0D,
                    c if (' '..='~').contains(&c) => (c as u8).to_ascii_uppercase(),
                    _ => continue,
                };
                self.paste_buf.push_back(b);
            }
        }

        if !in_logo_mode {
            for k in key_queue {
                if let Some(ref mut iigs) = self.iigs {
                    iigs.key_press(k);
                } else {
                    self.emu.bus.key_press(k);
                }
            }

            // Drain one character per frame from the paste buffer.
            // Only inject when the keyboard strobe has been cleared (bit 7 == 0),
            // which means the previous key has been read by the Apple II.
            let key_available = if let Some(ref iigs) = self.iigs {
                !iigs.bus.mega2.key_strobe
            } else {
                self.emu.bus.keyboard_data & 0x80 == 0
            };
            if key_available && let Some(k) = self.paste_buf.pop_front() {
                if let Some(ref mut iigs) = self.iigs {
                    iigs.key_press(k);
                } else {
                    self.emu.bus.key_press(k);
                }
            }

            // ── Joystick / paddle emulation ──────────────────────────────
            self.handle_joysticks(ctx);
        }

        // Screenshot: save framebuffer as BMP (triggered by F12).
        if take_screenshot && !in_logo_mode {
            self.render_apple2(); // ensure latest frame
            save_screenshot(self.fb.pixels_as_bytes(), SCREEN_W, SCREEN_H);
        }

        // Save / load emulator state (F11 / Shift+F11).
        if do_save_state && !in_logo_mode {
            if self.iigs.is_some() {
                self.status_msg = Some("Save state not yet available for Apple IIgs".to_string());
            } else if let Some(path) = self.config.save_state_path() {
                let snap = self.emu.take_snapshot();
                if let Ok(yaml) = serde_yaml::to_string(&snap) {
                    let _ = std::fs::write(&path, yaml);
                    self.status_msg = Some(format!("State saved to {}", path.display()));
                }
            }
        }
        if do_load_state && !in_logo_mode {
            if self.iigs.is_some() {
                self.status_msg = Some("Load state not yet available for Apple IIgs".to_string());
            } else if let Some(path) = self.config.save_state_path()
                && let Ok(yaml) = std::fs::read_to_string(&path)
                && let Ok(snap) = serde_yaml::from_str(&yaml)
            {
                self.emu.restore_snapshot(&snap);
                self.status_msg = Some("State loaded".to_string());
            }
        }

        // Speed control shortcuts (Ctrl+0/1/3).
        if let Some(spd) = speed_shortcut {
            self.config.emulation_speed = spd;
            self.config.save();
        }

        // WAV audio recording toggle (F9).
        if toggle_wav_rec {
            if let Some(rec) = self.wav_recorder.take() {
                let _ = rec.stop();
                self.status_msg = Some("Audio recording stopped".to_string());
            } else {
                let dir =
                    crate::config::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let path = dir.join(format!("applewin_audio_{ts}.wav"));
                match apple2_audio::wav_writer::WavRecorder::start(&path, self.audio_sample_rate) {
                    Ok(rec) => {
                        self.wav_recorder = Some(rec);
                        self.status_msg = Some(format!("Recording audio to {}", path.display()));
                    }
                    Err(e) => {
                        self.status_msg = Some(format!("WAV recording failed: {e}"));
                    }
                }
            }
        }

        // Alt key as Open / Closed Apple (button 0 / button 1).
        if self.config.alt_key_as_apple {
            ctx.input(|i| {
                alt_left = i.modifiers.alt;
                alt_right = false; // egui doesn't distinguish left/right Alt
            });
            // Map Alt to Open Apple (button 0 bit)
            if alt_left {
                self.emu.bus.gamepad.buttons |= 0x01;
            } else {
                self.emu.bus.gamepad.buttons &= !0x01;
            }
        }

        if do_hard_reset {
            self.reset(true);
        }
        if do_reset {
            self.reset(false);
        }
        // Quit is applied by the caller (frame.close() + early return).
        do_quit
    }
}

/// Map an egui `Key` + shift state to the Apple II ASCII byte for that key.
/// Letters are always returned as uppercase (Apple II convention).
/// Returns `None` for keys with no printable Apple II equivalent.
fn apple2_ascii_for_key(key: Key, shift: bool) -> Option<u8> {
    let c: u8 = match key {
        // Letters — always uppercase on Apple II
        Key::A => b'A',
        Key::B => b'B',
        Key::C => b'C',
        Key::D => b'D',
        Key::E => b'E',
        Key::F => b'F',
        Key::G => b'G',
        Key::H => b'H',
        Key::I => b'I',
        Key::J => b'J',
        Key::K => b'K',
        Key::L => b'L',
        Key::M => b'M',
        Key::N => b'N',
        Key::O => b'O',
        Key::P => b'P',
        Key::Q => b'Q',
        Key::R => b'R',
        Key::S => b'S',
        Key::T => b'T',
        Key::U => b'U',
        Key::V => b'V',
        Key::W => b'W',
        Key::X => b'X',
        Key::Y => b'Y',
        Key::Z => b'Z',
        // Digits and shifted symbols (standard US layout)
        Key::Num0 => {
            if shift {
                b')'
            } else {
                b'0'
            }
        }
        Key::Num1 => {
            if shift {
                b'!'
            } else {
                b'1'
            }
        }
        Key::Num2 => {
            if shift {
                b'@'
            } else {
                b'2'
            }
        }
        Key::Num3 => {
            if shift {
                b'#'
            } else {
                b'3'
            }
        }
        Key::Num4 => {
            if shift {
                b'$'
            } else {
                b'4'
            }
        }
        Key::Num5 => {
            if shift {
                b'%'
            } else {
                b'5'
            }
        }
        Key::Num6 => {
            if shift {
                b'^'
            } else {
                b'6'
            }
        }
        Key::Num7 => {
            if shift {
                b'&'
            } else {
                b'7'
            }
        }
        Key::Num8 => {
            if shift {
                b'*'
            } else {
                b'8'
            }
        }
        Key::Num9 => {
            if shift {
                b'('
            } else {
                b'9'
            }
        }
        Key::Space => b' ',
        _ => return None,
    };
    Some(c)
}
