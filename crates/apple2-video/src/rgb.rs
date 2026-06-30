//! RGB card / monitor renderer.
//!
//! Produces clean, direct-colour output without NTSC signal-chain artifacts.
//! Mirrors `source/RGBMonitor.cpp` from the C++ AppleWin.
//!
//! Differences from NTSC:
//! - Text uses the character ROM directly with fore/back colour from the RGB
//!   card's palette (default: white on black).
//! - Lo-res / double lo-res use the standard 16-colour palette.
//! - Hi-res is rendered per-pixel without NTSC colour fringing.
//! - Double hi-res maps each 4-bit nibble directly to one of 16 colours.

use crate::framebuffer::{FB_WIDTH, Framebuffer};
use crate::ntsc::CharRom;
use apple2_core::bus::MemMode;

// ── 16-colour ABGR palette (same order as NTSC lo-res) ──────────────────────

/// Standard Apple II 16-colour palette (ABGR8888), taken verbatim from
/// AppleWin's `RGBMonitor.cpp` "lores & dhires" table (Linards-tweaked values).
/// AppleWin source R,G,B listed in comments for direct verification.
const RGB_PALETTE: [u32; 16] = [
    0xFF000000, // 0: Black       00,00,00
    0xFF66099D, // 1: Deep Red    9D,09,66
    0xFFE52A2A, // 2: Dark Blue   2A,2A,E5
    0xFFFF34C7, // 3: Magenta     C7,34,FF
    0xFF008000, // 4: Dark Green  00,80,00
    0xFF808080, // 5: Dark Gray   80,80,80
    0xFFFFA10D, // 6: Blue        0D,A1,FF
    0xFFFFAAAA, // 7: Light Blue  AA,AA,FF
    0xFF005555, // 8: Brown       55,55,00
    0xFF005EF2, // 9: Orange      F2,5E,00
    0xFFC0C0C0, // A: Light Gray  C0,C0,C0
    0xFFE589FF, // B: Pink        FF,89,E5
    0xFF00CB38, // C: Green       38,CB,00
    0xFF1AD5D5, // D: Yellow      D5,D5,1A
    0xFF99F662, // E: Aqua        62,F6,99
    0xFFFFFFFF, // F: White       FF,FF,FF
];

/// Monochrome hi-res "colours" for RGB mode.  Bit set = white, bit clear = black.
/// The hi-bit (bit 7) has no effect in RGB mode (no colour fringing).
const HGR_WHITE: u32 = 0xFFFFFFFF;
const HGR_BLACK: u32 = 0xFF000000;

// ── RGB Renderer ─────────────────────────────────────────────────────────────

pub struct RgbRenderer {
    char_rom: CharRom,
    pub scanlines: bool,
    pub color_vertical_blend: bool,
}

impl RgbRenderer {
    pub fn new(char_rom: CharRom, scanlines: bool) -> Self {
        Self {
            char_rom,
            scanlines,
            color_vertical_blend: false,
        }
    }

    /// Render one complete RGB video frame.
    pub fn render(
        &self,
        main_ram: &[u8; 65536],
        aux_ram: &[u8; 65536],
        mode: MemMode,
        frame_no: u32,
        fb: &mut Framebuffer,
    ) {
        let flash_on = (frame_no / 16).is_multiple_of(2);

        let page2 = mode.contains(MemMode::MF_PAGE2);
        let graphics = mode.contains(MemMode::MF_GRAPHICS);
        let hires = mode.contains(MemMode::MF_HIRES);
        let mixed = mode.contains(MemMode::MF_MIXED);
        let vid80 = mode.contains(MemMode::MF_VID80);
        let dhires = mode.contains(MemMode::MF_DHIRES);
        let _store80 = mode.contains(MemMode::MF_80STORE);

        // Display page selection — same logic as NTSC renderer.
        let display_page2 = page2 && !_store80;
        let text_base: usize = if display_page2 { 0x0800 } else { 0x0400 };
        let hgr_base: usize = if display_page2 { 0x4000 } else { 0x2000 };
        let text_page = &main_ram[text_base..text_base + 0x400];

        if !graphics {
            if vid80 {
                self.render_text80_rows(main_ram, aux_ram, text_base, 0, 24, flash_on, fb);
            } else {
                self.render_text40_rows(text_page, 0, 24, flash_on, fb);
            }
        } else if hires {
            let scan_lines = if mixed { 160 } else { 192 };
            if dhires && vid80 {
                self.render_dhires(main_ram, aux_ram, hgr_base, scan_lines, fb);
            } else {
                self.render_hires(main_ram, hgr_base, scan_lines, fb);
            }
            if mixed {
                if vid80 {
                    self.render_text80_rows(main_ram, aux_ram, text_base, 20, 24, flash_on, fb);
                } else {
                    self.render_text40_rows(text_page, 20, 24, flash_on, fb);
                }
            }
        } else {
            if dhires && vid80 {
                self.render_dlores(main_ram, aux_ram, text_base, fb);
            } else {
                self.render_lores(text_page, fb);
            }
            if mixed {
                if vid80 {
                    self.render_text80_rows(main_ram, aux_ram, text_base, 20, 24, flash_on, fb);
                } else {
                    self.render_text40_rows(text_page, 20, 24, flash_on, fb);
                }
            }
        }

        if self.color_vertical_blend {
            crate::ntsc::apply_color_vertical_blend(fb);
        }
        if self.scanlines {
            crate::ntsc::apply_scanlines(fb);
        }
    }

    // ── Text rendering ───────────────────────────────────────────────────────

    fn render_text40_rows(
        &self,
        text_page: &[u8],
        row_start: usize,
        row_end: usize,
        flash_on: bool,
        fb: &mut Framebuffer,
    ) {
        let fg = 0xFFFFFFFF_u32; // white
        let bg = 0xFF000000_u32; // black
        for row in row_start..row_end {
            let base = crate::ntsc::text_row_offset(row);
            for col in 0..40 {
                let ch = text_page[base + col];
                let (glyph, invert) = self.char_rom.decode_char(ch, flash_on);
                let (f, b) = if invert { (bg, fg) } else { (fg, bg) };
                // Each character is 7 pixels wide × 8 scanlines high.
                // In 40-col mode, each pixel is doubled to 14 px (560 / 40 = 14).
                for (glyph_y, &row_byte) in glyph.iter().enumerate() {
                    let fb_y = row * 16 + glyph_y * 2; // 2× vertical
                    let fb_x_base = col * 14;
                    for bit in 0..7 {
                        let color = if (row_byte >> bit) & 1 != 0 { f } else { b };
                        let px = fb_x_base + bit * 2;
                        fb.set_pixel(px, fb_y, color);
                        fb.set_pixel(px + 1, fb_y, color);
                        fb.set_pixel(px, fb_y + 1, color);
                        fb.set_pixel(px + 1, fb_y + 1, color);
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_text80_rows(
        &self,
        main_ram: &[u8; 65536],
        aux_ram: &[u8; 65536],
        text_base: usize,
        row_start: usize,
        row_end: usize,
        flash_on: bool,
        fb: &mut Framebuffer,
    ) {
        let fg = 0xFFFFFFFF_u32;
        let bg = 0xFF000000_u32;
        for row in row_start..row_end {
            let base = crate::ntsc::text_row_offset(row);
            for col in 0..80 {
                let (ram, offset) = if col & 1 == 0 {
                    (aux_ram, text_base + base + col / 2)
                } else {
                    (main_ram, text_base + base + col / 2)
                };
                let ch = ram[offset];
                let (glyph, invert) = self.char_rom.decode_char(ch, flash_on);
                let (f, b) = if invert { (bg, fg) } else { (fg, bg) };
                for (glyph_y, &row_byte) in glyph.iter().enumerate() {
                    let fb_y = row * 16 + glyph_y * 2;
                    let fb_x_base = col * 7;
                    for bit in 0..7 {
                        let color = if (row_byte >> bit) & 1 != 0 { f } else { b };
                        fb.set_pixel(fb_x_base + bit, fb_y, color);
                        fb.set_pixel(fb_x_base + bit, fb_y + 1, color);
                    }
                }
            }
        }
    }

    // ── Lo-res rendering ─────────────────────────────────────────────────────

    fn render_lores(&self, text_page: &[u8], fb: &mut Framebuffer) {
        // Batch write: each lores cell is a 14-wide span of a single colour per
        // scan line, so we can use slice::fill to blast 14 pixels at once rather
        // than per-pixel set_pixel calls with bounds checks.
        let pixels = fb.pixels_mut();
        for row in 0..24 {
            let base = crate::ntsc::text_row_offset(row);
            for col in 0..40 {
                let byte = text_page[base + col];
                let top_color = RGB_PALETTE[(byte & 0x0F) as usize];
                let bot_color = RGB_PALETTE[((byte >> 4) & 0x0F) as usize];
                let fb_x_base = col * 14;
                let fb_y_base = row * 16;
                for y in 0..8 {
                    let color = if y < 4 { top_color } else { bot_color };
                    for line in 0..2 {
                        let fb_y = fb_y_base + y * 2 + line;
                        let row_start = fb_y * FB_WIDTH + fb_x_base;
                        pixels[row_start..row_start + 14].fill(color);
                    }
                }
            }
        }
    }

    fn render_dlores(
        &self,
        main_ram: &[u8; 65536],
        aux_ram: &[u8; 65536],
        text_base: usize,
        fb: &mut Framebuffer,
    ) {
        let pixels = fb.pixels_mut();
        for row in 0..24 {
            let base = crate::ntsc::text_row_offset(row);
            for col in 0..40 {
                let aux_byte = aux_ram[text_base + base + col];
                let main_byte = main_ram[text_base + base + col];
                let fb_x_base = col * 14;
                let fb_y_base = row * 16;
                for y in 0..8 {
                    let aux_color = if y < 4 {
                        RGB_PALETTE[(aux_byte & 0x0F) as usize]
                    } else {
                        RGB_PALETTE[((aux_byte >> 4) & 0x0F) as usize]
                    };
                    let main_color = if y < 4 {
                        RGB_PALETTE[(main_byte & 0x0F) as usize]
                    } else {
                        RGB_PALETTE[((main_byte >> 4) & 0x0F) as usize]
                    };
                    for line in 0..2 {
                        let fb_y = fb_y_base + y * 2 + line;
                        let row_start = fb_y * FB_WIDTH + fb_x_base;
                        pixels[row_start..row_start + 7].fill(aux_color);
                        pixels[row_start + 7..row_start + 14].fill(main_color);
                    }
                }
            }
        }
    }

    // ── Hi-res rendering ─────────────────────────────────────────────────────

    fn render_hires(
        &self,
        main_ram: &[u8; 65536],
        hgr_base: usize,
        scan_lines: usize,
        fb: &mut Framebuffer,
    ) {
        for y in 0..scan_lines {
            let addr = hgr_base + crate::ntsc::hgr_row_offset(y);
            for col in 0..40 {
                let byte = main_ram[addr + col];
                // In RGB mode, hi-res is monochrome: each bit → on/off pixel.
                // Bit 7 has no colour effect (no NTSC fringing).
                let fb_x_base = col * 14;
                let fb_y = y * 2;
                for bit in 0..7 {
                    let color = if (byte >> bit) & 1 != 0 {
                        HGR_WHITE
                    } else {
                        HGR_BLACK
                    };
                    fb.set_pixel(fb_x_base + bit * 2, fb_y, color);
                    fb.set_pixel(fb_x_base + bit * 2 + 1, fb_y, color);
                    fb.set_pixel(fb_x_base + bit * 2, fb_y + 1, color);
                    fb.set_pixel(fb_x_base + bit * 2 + 1, fb_y + 1, color);
                }
            }
        }
    }

    fn render_dhires(
        &self,
        main_ram: &[u8; 65536],
        aux_ram: &[u8; 65536],
        hgr_base: usize,
        scan_lines: usize,
        fb: &mut Framebuffer,
    ) {
        // Double hi-res: 4 bits per pixel, 16 colours, 140 pixels per line.
        // Each group of 7 aux bits + 7 main bits = 14 bits = 3.5 4-bit pixels.
        // Actually: pairs of (aux_byte, main_byte) give 28 bits = 7 pixels × 4 bits.
        for y in 0..scan_lines {
            let addr = hgr_base + crate::ntsc::hgr_row_offset(y);
            let fb_y = y * 2;
            for col_pair in 0..20 {
                // Read 2 aux bytes and 2 main bytes (28 bits from each pair).
                let a0 = aux_ram[addr + col_pair * 2] as u32;
                let m0 = main_ram[addr + col_pair * 2] as u32;
                let a1 = aux_ram[addr + col_pair * 2 + 1] as u32;
                let m1 = main_ram[addr + col_pair * 2 + 1] as u32;

                // Combine into a 28-bit value: a0[6:0] + m0[6:0] + a1[6:0] + m1[6:0]
                let bits =
                    (a0 & 0x7F) | ((m0 & 0x7F) << 7) | ((a1 & 0x7F) << 14) | ((m1 & 0x7F) << 21);

                let fb_x_base = col_pair * 28;
                for pixel in 0..7 {
                    let raw = (bits >> (pixel * 4)) & 0x0F;
                    // DHGR nibbles need AppleWin's DoubleHiresPalIndex remap
                    // (rotate-left-by-1) before indexing the 16-colour palette.
                    let nibble = (((raw << 1) | (raw >> 3)) & 0x0F) as usize;
                    let color = RGB_PALETTE[nibble];
                    let px = fb_x_base + pixel * 4;
                    for dx in 0..4 {
                        fb.set_pixel(px + dx, fb_y, color);
                        fb.set_pixel(px + dx, fb_y + 1, color);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framebuffer::{FB_HEIGHT, FB_WIDTH, Framebuffer};

    #[test]
    fn rgb_renderer_covers_all_video_modes() {
        // The RGB-card renderer must render every mode combination (no panic) and
        // fully cover the framebuffer.  It uses bounds-checked set_pixel, so this
        // primarily guards coverage and the DHGR palette path.
        use MemMode as M;
        let g = M::MF_GRAPHICS;
        let modes = [
            ("text40", M::empty()),
            ("text80", M::MF_VID80),
            ("lores", g),
            ("lores+mixed", g | M::MF_MIXED),
            ("dlores", g | M::MF_VID80 | M::MF_DHIRES),
            ("hires", g | M::MF_HIRES),
            ("hires+mixed", g | M::MF_HIRES | M::MF_MIXED),
            ("hires+page2", g | M::MF_HIRES | M::MF_PAGE2),
            ("dhires", g | M::MF_HIRES | M::MF_VID80 | M::MF_DHIRES),
            (
                "dhires+mixed",
                g | M::MF_HIRES | M::MF_VID80 | M::MF_DHIRES | M::MF_MIXED,
            ),
        ];

        let mut main_ram = Box::new([0u8; 65536]);
        let mut aux_ram = Box::new([0u8; 65536]);
        for i in 0..65536 {
            main_ram[i] = (i & 0xFF) as u8;
            aux_ram[i] = ((i >> 2) & 0xFF) as u8;
        }
        let renderer = RgbRenderer::new(CharRom::new(vec![0x3Cu8; 1024]), false);

        for (name, mode) in modes {
            let mut fb = Framebuffer::new();
            fb.pixels_mut().fill(0x0000_0000); // transparent sentinel
            renderer.render(&main_ram, &aux_ram, mode, 0, &mut fb);
            for fb_y in (0..FB_HEIGHT).step_by(2) {
                let row = fb_y * FB_WIDTH;
                let unwritten = fb.pixels()[row..row + FB_WIDTH]
                    .iter()
                    .filter(|&&p| p & 0xFF00_0000 == 0)
                    .count();
                assert_eq!(
                    unwritten, 0,
                    "RGB mode {name} row {fb_y}: {unwritten} unwritten"
                );
            }
        }
    }

    #[test]
    fn rgb_dhires_uses_palette_remap() {
        // DHGR nibble 3 → blue (lo-res 6), 12 → orange (9), via rotate-left-1.
        let renderer = RgbRenderer::new(CharRom::new(vec![0u8; 1024]), false);
        let main_ram = Box::new([0u8; 65536]);
        for (val, want) in [(3u8, RGB_PALETTE[6]), (12u8, RGB_PALETTE[9])] {
            let mut aux_ram = Box::new([0u8; 65536]);
            aux_ram[0x2000] = val;
            let mut fb = Framebuffer::new();
            renderer.render_dhires(&main_ram, &aux_ram, 0x2000, 192, &mut fb);
            assert_eq!(
                fb.pixels()[0],
                want,
                "rgb dhires nibble {val}: got {:#010x}",
                fb.pixels()[0]
            );
        }
    }
}
