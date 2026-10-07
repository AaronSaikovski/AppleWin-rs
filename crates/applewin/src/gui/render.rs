//! Video output: Apple II / IIgs frame rendering into the framebuffer,
//! debugger display rendering, GPU texture upload, and screenshots.

use super::*;

/// Main/aux RAM ranges the Apple II renderers read: text/lo-res pages 1–2
/// ($0400-$0BFF) and hi-res pages 1–2 ($2000-$5FFF).
const VRAM_RANGES: [std::ops::Range<usize>; 2] = [0x0400..0x0C00, 0x2000..0x6000];
/// Snapshot size: both ranges from both main and aux RAM.
const VRAM_SNAPSHOT_LEN: usize = 2 * (0x0800 + 0x4000);

/// Every non-RAM input that affects the Apple II rendered frame.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct FrameKey {
    mode_bits: u32,
    /// FLASH text phase; renderers toggle it every 16 frames.
    flash_phase: u32,
    video_type: VideoType,
    ntsc_tv_mode: bool,
    ntsc_scanlines: bool,
    ntsc_mono_tint: Option<[u8; 3]>,
    ntsc_vertical_blend: bool,
    rgb_scanlines: bool,
    rgb_vertical_blend: bool,
}

/// Inputs of the last rendered Apple II frame, so an unchanged screen can
/// skip rendering and the texture upload.
pub(super) struct FrameCache {
    key: Option<FrameKey>,
    /// Copy of the displayed video RAM (`VRAM_RANGES` of main, then aux).
    vram: Vec<u8>,
}

impl FrameCache {
    pub(super) fn new() -> Self {
        Self {
            key: None,
            vram: vec![0; VRAM_SNAPSHOT_LEN],
        }
    }

    /// Forget the cached frame so the next one is always rendered.
    pub(super) fn invalidate(&mut self) {
        self.key = None;
    }

    /// Returns `true` when `key` and the displayed video RAM match the
    /// previous call; otherwise records them and returns `false`.
    pub(super) fn is_unchanged(&mut self, key: FrameKey, main_ram: &[u8], aux_ram: &[u8]) -> bool {
        let mut unchanged = self.key == Some(key);
        self.key = Some(key);
        let mut offset = 0;
        for ram in [main_ram, aux_ram] {
            for range in VRAM_RANGES {
                let live = &ram[range];
                let saved = &mut self.vram[offset..offset + live.len()];
                offset += live.len();
                // Earlier regions matched, so after the first difference
                // only the remaining regions need refreshing.
                if !unchanged || saved != live {
                    unchanged = false;
                    saved.copy_from_slice(live);
                }
            }
        }
        unchanged
    }
}

impl EmulatorApp {
    fn frame_key(&self) -> FrameKey {
        FrameKey {
            mode_bits: self.emu.bus.mode.bits(),
            flash_phase: (self.frame_no / 16) % 2,
            video_type: self.config.video_type,
            ntsc_tv_mode: self.renderer.tv_mode,
            ntsc_scanlines: self.renderer.scanlines,
            ntsc_mono_tint: self.renderer.mono_tint,
            ntsc_vertical_blend: self.renderer.color_vertical_blend,
            rgb_scanlines: self.rgb_renderer.scanlines,
            rgb_vertical_blend: self.rgb_renderer.color_vertical_blend,
        }
    }

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
        if in_logo_mode {
            // The framebuffer isn't refreshed while the logo shows; make sure
            // the first frame after it is rendered.
            self.frame_cache.invalidate();
            return self.logo_texture.as_ref().map(|t| t.id());
        }

        // When debugger is active, render debugger into the framebuffer
        // instead of the Apple II screen, matching original AppleWin.
        // Neither the debugger view nor IIgs output is cached.
        if self.show_debugger && self.debugger.active {
            self.frame_cache.invalidate();
            self.render_debugger();
        } else if self.iigs.is_some() {
            self.frame_cache.invalidate();
            self.render_apple2();
        } else {
            let key = self.frame_key();
            let unchanged = self.frame_cache.is_unchanged(
                key,
                &self.emu.bus.main_ram[..],
                &self.emu.bus.aux_ram[..],
            );
            if unchanged && let Some(t) = &self.texture {
                return Some(t.id());
            }
            self.render_apple2();
        }

        let tex_opts = TextureOptions::NEAREST;
        // Every framebuffer pixel is opaque (alpha 0xFF), so premultiplied
        // and unmultiplied RGBA are identical; the premultiplied path skips
        // epaint's per-pixel unmultiply (~126 µs vs ~10 µs per frame).
        let image =
            ColorImage::from_rgba_premultiplied([SCREEN_W, SCREEN_H], self.fb.pixels_as_bytes());
        if let Some(t) = &mut self.texture {
            t.set(image, tex_opts);
        } else {
            self.texture = Some(ctx.load_texture("apple2", image, tex_opts));
        }
        self.texture.as_ref().map(|t| t.id())
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
