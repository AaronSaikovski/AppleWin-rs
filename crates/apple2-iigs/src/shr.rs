//! Apple IIgs Super Hi-Res (SHR) video renderer.
//!
//! The SHR display lives in fast RAM bank $E1:
//! - $2000-$9CFF: Pixel data (200 scanlines × 160 bytes = 32,000 bytes)
//! - $9D00-$9DC7: Scanline Control Bytes (SCBs, 200 bytes)
//! - $9E00-$9FFF: Color palettes (16 palettes × 16 entries × 2 bytes = 512 bytes)
//!
//! Two modes per scanline (selected by SCB):
//! - **320 mode**: 160 bytes/line, each byte = 2 pixels (4 bits each), 16 colors/line
//! - **640 mode**: 160 bytes/line, each byte = 4 pixels (2 bits each), 4 colors/line
//!
//! Each scanline independently selects its palette (0-15) and mode (320/640).

/// SHR framebuffer dimensions.
pub const SHR_WIDTH: usize = 640;
pub const SHR_HEIGHT: usize = 400; // 200 lines × 2 (pixel doubled vertically)

/// Offsets within bank $E1 fast RAM.
const PIXEL_BASE: usize = 0x2000;
const SCB_BASE: usize = 0x9D00;
const PALETTE_BASE: usize = 0x9E00;

/// Scanline Control Byte bit definitions.
const SCB_MODE_640: u8 = 0x80; // Bit 7: 1 = 640 mode, 0 = 320 mode
const SCB_FILL: u8 = 0x20; // Bit 5: fill mode (use previous line's data)
#[allow(dead_code)]
const SCB_INTERRUPT: u8 = 0x40; // Bit 6: scanline interrupt (not used for rendering)

/// Convert a 4-bit IIgs color component (0-15) to 8-bit (0-255).
#[inline]
fn expand_4to8(val: u8) -> u8 {
    val | (val << 4)
}

/// Convert a 16-bit IIgs palette entry ($0RGB, 4 bits per component) to ABGR8888.
#[inline]
fn palette_to_abgr(lo: u8, hi: u8) -> u32 {
    let r = expand_4to8(hi & 0x0F);
    let g = expand_4to8(lo >> 4);
    let b = expand_4to8(lo & 0x0F);
    0xFF00_0000 | (b as u32) << 16 | (g as u32) << 8 | r as u32
}

/// Render the SHR display from bank $E1 fast RAM into an ABGR pixel buffer.
///
/// `fast_ram_e1`: Slice of bank $E1 data (65536 bytes, offset 0 = address $0000).
/// `pixels`: Output buffer, must be at least `SHR_WIDTH * SHR_HEIGHT` u32 entries.
///           Pixels are stored row-major, ABGR8888 format.
pub fn render_shr(fast_ram_e1: &[u8], pixels: &mut [u32]) {
    if fast_ram_e1.len() < 0xA000 || pixels.len() < SHR_WIDTH * SHR_HEIGHT {
        return;
    }

    // Pre-decode all 16 palettes (16 entries each, 256 colors total)
    let mut palette_cache = [[0u32; 16]; 16];
    for (pal_idx, pal) in palette_cache.iter_mut().enumerate() {
        for (entry, color) in pal.iter_mut().enumerate() {
            let offset = PALETTE_BASE + pal_idx * 32 + entry * 2;
            let lo = fast_ram_e1[offset];
            let hi = fast_ram_e1[offset + 1];
            *color = palette_to_abgr(lo, hi);
        }
    }

    for line in 0..200 {
        let scb = fast_ram_e1[SCB_BASE + line];
        let pal_idx = (scb & 0x0F) as usize;
        let is_640 = scb & SCB_MODE_640 != 0;
        let is_fill = scb & SCB_FILL != 0;
        let palette = &palette_cache[pal_idx];

        let pixel_offset = PIXEL_BASE + line * 160;
        let out_y = line * 2; // pixel-double vertically

        if is_fill && line > 0 {
            // Fill mode: copy the previous scanline
            let prev_start = (out_y - 2) * SHR_WIDTH;
            let cur_start = out_y * SHR_WIDTH;
            // Copy within the pixel buffer
            for x in 0..SHR_WIDTH {
                pixels[cur_start + x] = pixels[prev_start + x];
            }
        } else if is_640 {
            // 640 mode: 4 pixels per byte (2 bits each), 4 colors per scanline
            // Bits 7-6 = pixel 0, 5-4 = pixel 1, 3-2 = pixel 2, 1-0 = pixel 3
            // Colors come from palette entries 0-3 only
            let out_start = out_y * SHR_WIDTH;
            for byte_idx in 0..160 {
                let byte = fast_ram_e1[pixel_offset + byte_idx];
                let x = byte_idx * 4;
                // 640 mode: each pixel is displayed at native resolution (no doubling)
                pixels[out_start + x] = palette[((byte >> 6) & 0x03) as usize];
                pixels[out_start + x + 1] = palette[((byte >> 4) & 0x03) as usize];
                pixels[out_start + x + 2] = palette[((byte >> 2) & 0x03) as usize];
                pixels[out_start + x + 3] = palette[(byte & 0x03) as usize];
            }
        } else {
            // 320 mode: 2 pixels per byte (4 bits each), 16 colors per scanline
            // High nibble = left pixel, low nibble = right pixel
            // Each pixel is doubled horizontally to fill 640 width
            let out_start = out_y * SHR_WIDTH;
            for byte_idx in 0..160 {
                let byte = fast_ram_e1[pixel_offset + byte_idx];
                let left = palette[((byte >> 4) & 0x0F) as usize];
                let right = palette[(byte & 0x0F) as usize];
                let x = byte_idx * 4; // 2 pixels × 2 (doubled) = 4 output pixels
                pixels[out_start + x] = left;
                pixels[out_start + x + 1] = left;
                pixels[out_start + x + 2] = right;
                pixels[out_start + x + 3] = right;
            }
        }

        // Pixel-double the scanline vertically
        let src_start = out_y * SHR_WIDTH;
        let dst_start = (out_y + 1) * SHR_WIDTH;
        for x in 0..SHR_WIDTH {
            pixels[dst_start + x] = pixels[src_start + x];
        }
    }
}

/// Render the SHR border color as a solid fill.
/// `border_color`: 4-bit color index from palette 0.
pub fn border_color_abgr(fast_ram_e1: &[u8], border_idx: u8) -> u32 {
    let entry = (border_idx & 0x0F) as usize;
    let offset = PALETTE_BASE + entry * 2;
    if offset + 1 < fast_ram_e1.len() {
        palette_to_abgr(fast_ram_e1[offset], fast_ram_e1[offset + 1])
    } else {
        0xFF00_0000 // black
    }
}
