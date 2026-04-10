//! 560×384 ABGR8888 framebuffer.
//!
//! Pixel encoding: ABGR8888 native-endian `u32` — R in the low byte, A in the
//! high byte.  On little-endian (x86/ARM) the in-memory byte order is
//! `[R, G, B, A]`, identical to egui's `from_rgba_unmultiplied` format.
//! `pixels_as_bytes()` therefore returns upload-ready RGBA data with no
//! per-frame channel conversion.
//!
//! Matches the dimensions used by `GetFrameBuffer()` in `source/Video.cpp`.

pub const FB_WIDTH: usize = 560;
pub const FB_HEIGHT: usize = 384;
pub const FB_PIXELS: usize = FB_WIDTH * FB_HEIGHT;

/// ABGR8888 framebuffer (in-memory bytes are RGBA on little-endian).
pub struct Framebuffer {
    pixels: Box<[u32; FB_PIXELS]>,
}

impl Framebuffer {
    pub fn new() -> Self {
        Self {
            pixels: Box::new([0xFF000000u32; FB_PIXELS]),
        }
    }

    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, argb: u32) {
        if x < FB_WIDTH && y < FB_HEIGHT {
            self.pixels[y * FB_WIDTH + x] = argb;
        }
    }

    #[inline]
    pub fn pixels(&self) -> &[u32; FB_PIXELS] {
        &self.pixels
    }

    #[inline]
    pub fn pixels_mut(&mut self) -> &mut [u32; FB_PIXELS] {
        &mut self.pixels
    }

    #[inline]
    pub fn pixels_as_bytes(&self) -> &[u8] {
        // SAFETY: [u32; N] has the same size as [u8; N*4] with no padding.
        unsafe { std::slice::from_raw_parts(self.pixels.as_ptr() as *const u8, FB_PIXELS * 4) }
    }

    pub fn clear(&mut self, argb: u32) {
        self.pixels.fill(argb);
    }
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}
