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

use apple2_core::bus::MemMode;
use crate::framebuffer::Framebuffer;
use crate::ntsc::CharRom;

// ── 16-colour ABGR palette (same order as NTSC lo-res) ──────────────────────

/// Standard Apple II 16-colour palette (ABGR8888).
const RGB_PALETTE: [u32; 16] = [
    0xFF000000, // 0: Black
    0xFF12129D, // 1: Deep Red
    0xFF990011, // 2: Dark Blue
    0xFFBB22AA, // 3: Purple
    0xFF226600, // 4: Dark Green
    0xFF6A6A6A, // 5: Dark Gray
    0xFFFF2222, // 6: Medium Blue
    0xFFEE9955, // 7: Light Blue
    0xFF004466, // 8: Brown
    0xFF0066FF, // 9: Orange
    0xFF999999, // A: Light Gray
    0xFFAA99FF, // B: Pink
    0xFF11DD11, // C: Light Green
    0xFF00FFFF, // D: Yellow
    0xFFAAFF33, // E: Aqua
    0xFFFFFFFF, // F: White
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
        aux_ram:  &[u8; 65536],
        mode: MemMode,
        frame_no: u32,
        fb: &mut Framebuffer,
    ) {
        let flash_on = (frame_no / 16).is_multiple_of(2);

        let page2    = mode.contains(MemMode::MF_PAGE2);
        let graphics = mode.contains(MemMode::MF_GRAPHICS);
        let hires    = mode.contains(MemMode::MF_HIRES);
        let mixed    = mode.contains(MemMode::MF_MIXED);
        let vid80    = mode.contains(MemMode::MF_VID80);
        let dhires   = mode.contains(MemMode::MF_DHIRES);
        let _store80 = mode.contains(MemMode::MF_80STORE);

        // Display page selection — same logic as NTSC renderer.
        let display_page2 = page2 && !_store80;
        let text_base: usize = if display_page2 { 0x0800 } else { 0x0400 };
        let hgr_base:  usize = if display_page2 { 0x4000 } else { 0x2000 };
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
        &self, text_page: &[u8], row_start: usize, row_end: usize,
        flash_on: bool, fb: &mut Framebuffer,
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
        &self, main_ram: &[u8; 65536], aux_ram: &[u8; 65536],
        text_base: usize, row_start: usize, row_end: usize,
        flash_on: bool, fb: &mut Framebuffer,
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
        for row in 0..24 {
            let base = crate::ntsc::text_row_offset(row);
            for col in 0..40 {
                let byte = text_page[base + col];
                let top_color = RGB_PALETTE[(byte & 0x0F) as usize];
                let bot_color = RGB_PALETTE[((byte >> 4) & 0x0F) as usize];
                // Each lo-res block is 14×8 pixels (doubled for 560×384).
                let fb_x_base = col * 14;
                let fb_y_base = row * 16;
                for y in 0..8 {
                    let color = if y < 4 { top_color } else { bot_color };
                    for x in 0..14 {
                        fb.set_pixel(fb_x_base + x, fb_y_base + y * 2, color);
                        fb.set_pixel(fb_x_base + x, fb_y_base + y * 2 + 1, color);
                    }
                }
            }
        }
    }

    fn render_dlores(
        &self, main_ram: &[u8; 65536], aux_ram: &[u8; 65536],
        text_base: usize, fb: &mut Framebuffer,
    ) {
        for row in 0..24 {
            let base = crate::ntsc::text_row_offset(row);
            for col in 0..40 {
                let aux_byte  = aux_ram[text_base + base + col];
                let main_byte = main_ram[text_base + base + col];
                // Double lo-res: each column produces two 7-pixel-wide colour blocks.
                // Aux provides the left column, main the right.
                let fb_x_base = col * 14;
                let fb_y_base = row * 16;
                for y in 0..8 {
                    let aux_color  = if y < 4 {
                        RGB_PALETTE[(aux_byte & 0x0F) as usize]
                    } else {
                        RGB_PALETTE[((aux_byte >> 4) & 0x0F) as usize]
                    };
                    let main_color = if y < 4 {
                        RGB_PALETTE[(main_byte & 0x0F) as usize]
                    } else {
                        RGB_PALETTE[((main_byte >> 4) & 0x0F) as usize]
                    };
                    for x in 0..7 {
                        fb.set_pixel(fb_x_base + x, fb_y_base + y * 2, aux_color);
                        fb.set_pixel(fb_x_base + x, fb_y_base + y * 2 + 1, aux_color);
                        fb.set_pixel(fb_x_base + 7 + x, fb_y_base + y * 2, main_color);
                        fb.set_pixel(fb_x_base + 7 + x, fb_y_base + y * 2 + 1, main_color);
                    }
                }
            }
        }
    }

    // ── Hi-res rendering ─────────────────────────────────────────────────────

    fn render_hires(&self, main_ram: &[u8; 65536], hgr_base: usize, scan_lines: usize, fb: &mut Framebuffer) {
        for y in 0..scan_lines {
            let addr = hgr_base + crate::ntsc::hgr_row_offset(y);
            for col in 0..40 {
                let byte = main_ram[addr + col];
                // In RGB mode, hi-res is monochrome: each bit → on/off pixel.
                // Bit 7 has no colour effect (no NTSC fringing).
                let fb_x_base = col * 14;
                let fb_y = y * 2;
                for bit in 0..7 {
                    let color = if (byte >> bit) & 1 != 0 { HGR_WHITE } else { HGR_BLACK };
                    fb.set_pixel(fb_x_base + bit * 2, fb_y, color);
                    fb.set_pixel(fb_x_base + bit * 2 + 1, fb_y, color);
                    fb.set_pixel(fb_x_base + bit * 2, fb_y + 1, color);
                    fb.set_pixel(fb_x_base + bit * 2 + 1, fb_y + 1, color);
                }
            }
        }
    }

    fn render_dhires(
        &self, main_ram: &[u8; 65536], aux_ram: &[u8; 65536],
        hgr_base: usize, scan_lines: usize, fb: &mut Framebuffer,
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
                let bits = (a0 & 0x7F)
                    | ((m0 & 0x7F) << 7)
                    | ((a1 & 0x7F) << 14)
                    | ((m1 & 0x7F) << 21);

                let fb_x_base = col_pair * 28;
                for pixel in 0..7 {
                    let nibble = ((bits >> (pixel * 4)) & 0x0F) as usize;
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
