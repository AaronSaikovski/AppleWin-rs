//! Video output: Apple II / IIgs frame rendering into the framebuffer,
//! debugger display rendering, GPU texture upload, and screenshots.

use super::*;

impl EmulatorApp {
    /// Render the Apple II screen into `self.pixel_buf` via `NtscRenderer`.
    pub(super) fn render_apple2(&mut self) {
        if let Some(ref iigs) = self.iigs {
            // Check if SHR mode is enabled (NEWVIDEO bit 7)
            if iigs.bus.mega2.is_shr_enabled() {
                // SHR rendering: use bank $E1 fast RAM
                let fast = &iigs.bus.mem.fast_ram;
                if fast.len() >= 0x20000 {
                    let bank_e1 = &fast[0x10000..0x20000];
                    // Render SHR into the reusable scratch buffer (640x400 → scale to 560x384).
                    apple2_iigs::shr::render_shr(bank_e1, &mut self.shr_scratch);
                    // Scale SHR (640x400) to the display framebuffer (560x384).
                    // Move scratch out to satisfy the borrow checker (scale_shr_to_framebuffer
                    // takes &mut self), then put it back — no allocation.
                    let shr = std::mem::take(&mut self.shr_scratch);
                    self.scale_shr_to_framebuffer(&shr);
                    self.shr_scratch = shr;
                }
                return;
            }

            // IIe-compatible mode: copy fast RAM to dummy emulator for rendering
            let fast = &iigs.bus.mem.fast_ram;
            let main_end = 0x10000.min(fast.len());
            self.emu.bus.main_ram[..main_end].copy_from_slice(&fast[..main_end]);
            if fast.len() >= 0x20000 {
                self.emu
                    .bus
                    .aux_ram
                    .copy_from_slice(&fast[0x10000..0x20000]);
            }
            self.emu.bus.mode = iigs.bus.mega2.mem_mode;
        }

        match self.config.video_type {
            crate::config::VideoType::ColorRGB => {
                self.rgb_renderer.render(
                    &self.emu.bus.main_ram,
                    &self.emu.bus.aux_ram,
                    self.emu.bus.mode,
                    self.frame_no,
                    &mut self.fb,
                );
            }
            crate::config::VideoType::ColorIdealized => {
                self.renderer.render_idealized(
                    &self.emu.bus.main_ram,
                    &self.emu.bus.aux_ram,
                    self.emu.bus.mode,
                    self.frame_no,
                    &mut self.fb,
                );
            }
            _ => {
                self.renderer.render(
                    &self.emu.bus.main_ram,
                    &self.emu.bus.aux_ram,
                    self.emu.bus.mode,
                    self.frame_no,
                    &mut self.fb,
                );
            }
        }
    }

    /// Scale a 640×400 SHR pixel buffer down to the 560×384 framebuffer.
    ///
    /// Uses precomputed source-coordinate lookup tables
    /// (`SHR_SRC_X` / `SHR_SRC_Y`) to avoid per-pixel division.
    fn scale_shr_to_framebuffer(&mut self, shr_pixels: &[u32]) {
        const SRC_W: usize = 640;
        let fb_pixels = self.fb.pixels_mut();
        for (dst_y, &src_y) in SHR_SRC_Y.iter().enumerate() {
            let src_row_base = src_y as usize * SRC_W;
            let dst_row_base = dst_y * SCREEN_W;
            for (dst_x, &src_x) in SHR_SRC_X.iter().enumerate() {
                fb_pixels[dst_row_base + dst_x] = shr_pixels[src_row_base + src_x as usize];
            }
        }
    }

    /// Render the debugger display into the framebuffer, replacing the
    /// Apple II screen — matching the original AppleWin behaviour.
    pub(super) fn render_debugger(&mut self) {
        use apple2_debugger::display::{self, CpuSnapshot};

        let (cpu, mode_bits) = if let Some(ref iigs) = self.iigs {
            (
                CpuSnapshot {
                    pc: iigs.cpu.pc,
                    a: (iigs.cpu.c & 0xFF) as u8,
                    x: (iigs.cpu.x & 0xFF) as u8,
                    y: (iigs.cpu.y & 0xFF) as u8,
                    sp: (iigs.cpu.sp & 0xFF) as u8,
                    flags: iigs.cpu.flags.bits(),
                    cycles: iigs.cpu.cycles,
                },
                iigs.bus.mega2.mem_mode.bits(),
            )
        } else {
            (
                CpuSnapshot {
                    pc: self.emu.cpu.pc,
                    a: self.emu.cpu.a,
                    x: self.emu.cpu.x,
                    y: self.emu.cpu.y,
                    sp: self.emu.cpu.sp,
                    flags: self.emu.cpu.flags.bits(),
                    cycles: self.emu.cpu.cycles,
                },
                self.emu.bus.mode.bits(),
            )
        };

        if let Some(ref iigs) = self.iigs {
            let iigs_ref = iigs;
            display::render(
                self.fb.pixels_mut(),
                &self.debugger,
                &cpu,
                mode_bits,
                &self.debugger_cmd_input,
                |a| iigs_ref.bus.mem.ram_read(0, a),
            );
        } else {
            display::render(
                self.fb.pixels_mut(),
                &self.debugger,
                &cpu,
                mode_bits,
                &self.debugger_cmd_input,
                |a| self.emu.bus.read_raw(a),
            );
        }
    }
    pub(super) fn upload_frame_texture(
        &mut self,
        ctx: &egui::Context,
        in_logo_mode: bool,
    ) -> Option<egui::TextureId> {
        // ── Render display to GPU texture ─────────────────────────────────
        // When debugger is active, render debugger into the framebuffer
        // instead of the Apple II screen, matching original AppleWin.
        if !in_logo_mode {
            if self.show_debugger && self.debugger.active {
                self.render_debugger();
            } else {
                self.render_apple2();
            }

            let tex_opts = TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Nearest,
            };
            let image =
                ColorImage::from_rgba_unmultiplied([SCREEN_W, SCREEN_H], self.fb.pixels_as_bytes());
            if let Some(t) = &mut self.texture {
                t.set(image, tex_opts);
            } else {
                self.texture = Some(ctx.load_texture("apple2", image, tex_opts));
            }
        }
        if in_logo_mode {
            self.logo_texture.as_ref().map(|t| t.id())
        } else {
            self.texture.as_ref().map(|t| t.id())
        }
    }
}
// ── Screenshot ────────────────────────────────────────────────────────────

/// Save `pixels` (RGBA8888, row-major) as a PNG file.
pub(super) fn save_screenshot(pixels: &[u8], w: usize, h: usize) {
    let Some(path) = screenshot_path() else {
        return;
    };

    let Ok(file) = std::fs::File::create(&path) else {
        eprintln!("Screenshot failed: could not create {}", path.display());
        return;
    };
    let buf_writer = &mut std::io::BufWriter::new(file);

    let mut encoder = png::Encoder::new(buf_writer, w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);

    let Ok(mut writer) = encoder.write_header() else {
        eprintln!("Screenshot failed: could not write PNG header");
        return;
    };

    if writer.write_image_data(pixels).is_ok() {
        eprintln!("Screenshot saved: {}", path.display());
    } else {
        eprintln!("Screenshot failed: could not write PNG data");
    }
}

/// Returns a timestamped path in %APPDATA%\applewin-rs\screenshots\ (Windows)
/// or the current directory (other platforms).
fn screenshot_path() -> Option<PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let fname = format!("screenshot_{ts}.png");

    #[cfg(windows)]
    {
        let appdata = std::env::var_os("APPDATA")?;
        let dir = PathBuf::from(appdata)
            .join("applewin-rs")
            .join("screenshots");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir.join(fname))
    }
    #[cfg(not(windows))]
    {
        Some(PathBuf::from(fname))
    }
}
// ── Apple IIe character ROM ────────────────────────────────────────────────
//
// 4 KB video ROM: glyphs 0x00-0x3F live at 0x0400, glyphs 0x40-0x7F at 0x0600.
// Each glyph is 8 consecutive bytes.  Raw bytes are stored inverted (0=lit).
// Per UTAIIe §8-30, the ROM bit order is also reversed: bit 0 = leftmost pixel.
// We XOR-invert then bit-reverse bits [6:0] and shift left by 1 to produce our
// MSB-first format (bit 7 = leftmost pixel).
pub(super) static VIDEO_ROM: &[u8] = include_bytes!("../../../../roms/Apple2e_Enhanced_Video.rom");

pub(super) fn build_font_from_rom(rom: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(128 * 8);
    for idx in 0u8..128 {
        let base = if idx < 64 {
            0x0400 + (idx as usize) * 8
        } else {
            0x0600 + (idx as usize - 64) * 8
        };
        for row in 0..8 {
            let inv = rom[base + row] ^ 0xFF; // invert polarity: 1 = lit pixel
            // Bit-reverse bits [6:0]: ROM bit 0 = leftmost → our bit 7 = leftmost
            let mut d: u8 = 0;
            let mut n = inv;
            for _ in 0..7 {
                d = (d << 1) | (n & 1);
                n >>= 1;
            }
            out.push(d << 1); // shift into bits [7:1]; bit 7 = leftmost pixel
        }
    }
    out
}
