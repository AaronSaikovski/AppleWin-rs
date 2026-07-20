//! Embedded BMP icon assets, BMP decoding, and small chrome widgets
//! (icon buttons, disk LEDs, Windows 9x-style sunken bevel).

use super::*;

// ── Embedded toolbar BMP icons ────────────────────────────────────────────

pub(super) static BMP_HELP: &[u8] = include_bytes!("../../icons/HELP.BMP");
pub(super) static BMP_RUN: &[u8] = include_bytes!("../../icons/RUN.BMP");
pub(super) static BMP_D1: &[u8] = include_bytes!("../../icons/DRIVE1.BMP");
pub(super) static BMP_D2: &[u8] = include_bytes!("../../icons/DRIVE2.BMP");
pub(super) static BMP_SWAP: &[u8] = include_bytes!("../../icons/DriveSwap.bmp");
pub(super) static BMP_FULL: &[u8] = include_bytes!("../../icons/FULLSCR.BMP");
pub(super) static BMP_DEBUG: &[u8] = include_bytes!("../../icons/DEBUG.BMP");
pub(super) static BMP_SETUP: &[u8] = include_bytes!("../../icons/SETUP.BMP");
pub(super) static BMP_LOGO: &[u8] = include_bytes!("../../icons/ApplewinLogo.bmp");

/// Decode a Windows indexed-colour BMP (4bpp or 8bpp) to RGBA8888 pixels.
///
/// Cyan (0, 255, 255) is treated as fully transparent (chroma-key).
pub(super) fn decode_bmp_rgba(data: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    if data.len() < 54 || &data[0..2] != b"BM" {
        return None;
    }
    let pixel_offset = u32::from_le_bytes(data[10..14].try_into().ok()?) as usize;
    let w = i32::from_le_bytes(data[18..22].try_into().ok()?) as usize;
    let h_raw = i32::from_le_bytes(data[22..26].try_into().ok()?);
    let h = h_raw.unsigned_abs() as usize;
    let bpp = u16::from_le_bytes(data[28..30].try_into().ok()?);
    let colors_used = u32::from_le_bytes(data[46..50].try_into().ok()?) as usize;

    let num_colors: usize = match bpp {
        4 => {
            if colors_used > 0 {
                colors_used
            } else {
                16
            }
        }
        8 => {
            if colors_used > 0 {
                colors_used
            } else {
                256
            }
        }
        _ => return None,
    };

    // Build palette: BMP stores RGBQUAD as (blue, green, red, reserved)
    let pal_start = 54usize;
    if data.len() < pal_start + num_colors * 4 {
        return None;
    }
    let mut palette = Vec::with_capacity(num_colors);
    for i in 0..num_colors {
        let b = data[pal_start + i * 4];
        let g = data[pal_start + i * 4 + 1];
        let r = data[pal_start + i * 4 + 2];
        palette.push((r, g, b));
    }

    let flip = h_raw > 0; // positive height = bottom-to-top storage
    let row_stride = match bpp {
        4 => (w * 4).div_ceil(32) * 4,
        8 => w.div_ceil(4) * 4,
        _ => return None,
    };

    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        let src_row = if flip { h - 1 - row } else { row };
        let src_off = pixel_offset + src_row * row_stride;
        for x in 0..w {
            let idx: usize = match bpp {
                4 => {
                    let byte = *data.get(src_off + x / 2)?;
                    if x % 2 == 0 {
                        (byte >> 4) as usize
                    } else {
                        (byte & 0xF) as usize
                    }
                }
                8 => *data.get(src_off + x)? as usize,
                _ => return None,
            };
            let (r, g, b) = *palette.get(idx)?;
            // Cyan (0, 255, 255) is the chroma-key colour used by Win32 toolbar
            let a = if r == 0 && g == 255 && b == 255 {
                0u8
            } else {
                255u8
            };
            let dst = (row * w + x) * 4;
            rgba[dst] = r;
            rgba[dst + 1] = g;
            rgba[dst + 2] = b;
            rgba[dst + 3] = a;
        }
    }
    Some((w, h, rgba))
}

/// Decode a 24bpp Windows BMP to RGBA8888 pixels.
pub(super) fn decode_bmp24_rgba(data: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    if data.len() < 54 || &data[0..2] != b"BM" {
        return None;
    }
    let pixel_offset = u32::from_le_bytes(data[10..14].try_into().ok()?) as usize;
    let w = i32::from_le_bytes(data[18..22].try_into().ok()?) as usize;
    let h_raw = i32::from_le_bytes(data[22..26].try_into().ok()?);
    let h = h_raw.unsigned_abs() as usize;
    let bpp = u16::from_le_bytes(data[28..30].try_into().ok()?);
    if bpp != 24 {
        return None;
    }
    let flip = h_raw > 0;
    let row_stride = (w * 3).div_ceil(4) * 4;
    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        let src_row = if flip { h - 1 - row } else { row };
        let src_off = pixel_offset + src_row * row_stride;
        for x in 0..w {
            let b = *data.get(src_off + x * 3)?;
            let g = *data.get(src_off + x * 3 + 1)?;
            let r = *data.get(src_off + x * 3 + 2)?;
            let dst = (row * w + x) * 4;
            rgba[dst] = r;
            rgba[dst + 1] = g;
            rgba[dst + 2] = b;
            rgba[dst + 3] = 255;
        }
    }
    Some((w, h, rgba))
}

// ── Widget helpers ────────────────────────────────────────────────────────

/// Toolbar button: raised Win9x bevel with a BMP icon centred inside.
///
/// Falls back to a labelled button if the texture is not available.
pub(super) fn icon_btn(
    ui: &mut egui::Ui,
    tex: Option<&egui::TextureHandle>,
    fallback: &str,
    tooltip: &str,
    btn_size: Vec2,
    img_size: Vec2,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(btn_size, Sense::click());

    // Background fill
    let bg = if resp.is_pointer_button_down_on() {
        Color32::from_rgb(180, 178, 170) // slightly darker when pressed
    } else {
        WIN_FACE
    };
    ui.painter().rect_filled(rect, 2.0, bg);

    // Raised bevel (inverts when pressed)
    let (tl, br) = if resp.is_pointer_button_down_on() {
        (WIN_SHADOW, WIN_LIGHT)
    } else {
        (WIN_LIGHT, WIN_SHADOW)
    };
    let w = 1.0f32;
    let p = ui.painter();
    // top
    p.line_segment([rect.left_top(), rect.right_top()], Stroke::new(w, tl));
    // left
    p.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(w, tl));
    // bottom
    p.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(w, br),
    );
    // right
    p.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(w, br));

    if let Some(t) = tex {
        // Centre the icon image inside the button
        let offset = if resp.is_pointer_button_down_on() {
            Vec2::new(1.0, 1.0)
        } else {
            Vec2::ZERO
        };
        let img_rect = Rect::from_center_size(rect.center() + offset, img_size);
        ui.painter().image(
            t.id(),
            img_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        // Fallback: text label centred in the button
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            fallback,
            FontId::proportional(17.0),
            Color32::BLACK,
        );
    }

    if resp.hovered() {
        ui.painter()
            .rect_stroke(rect, 2.0, Stroke::new(1.0_f32, WIN_DSHADOW));
    }

    resp.on_hover_text(tooltip)
}

/// Coloured disk-activity LED widget.
pub(super) fn disk_led(ui: &mut egui::Ui, label: &str, active: bool, writing: bool) {
    let color = if !active {
        Color32::from_rgb(25, 25, 25)
    } else if writing {
        Color32::from_rgb(210, 40, 40)
    } else {
        Color32::from_rgb(40, 200, 40)
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(26.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, color);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(9.0),
        Color32::WHITE,
    );
}
// ── 3D sunken bevel (Windows 9x "SunkenBox" look) ─────────────────────────

pub(super) fn draw_sunken_bevel(painter: &egui::Painter, outer: Rect) {
    // Outer ring: dark shadow top/left, highlight bottom/right
    bevel_ring(painter, outer, 2.0, WIN_DSHADOW, WIN_LIGHT);
    // Inner ring: mid-shadow top/left, face-colour bottom/right
    bevel_ring(painter, outer.shrink(2.0), 2.0, WIN_SHADOW, WIN_HILIGHT);
}

/// Draw top+left edges in `tl` colour and bottom+right edges in `br` colour.
fn bevel_ring(painter: &egui::Painter, r: Rect, w: f32, tl: Color32, br: Color32) {
    let h = w / 2.0;
    let stl = Stroke::new(w, tl);
    let sbr = Stroke::new(w, br);
    // top
    painter.line_segment(
        [
            Pos2::new(r.left(), r.top() + h),
            Pos2::new(r.right(), r.top() + h),
        ],
        stl,
    );
    // left
    painter.line_segment(
        [
            Pos2::new(r.left() + h, r.top()),
            Pos2::new(r.left() + h, r.bottom()),
        ],
        stl,
    );
    // bottom
    painter.line_segment(
        [
            Pos2::new(r.left(), r.bottom() - h),
            Pos2::new(r.right(), r.bottom() - h),
        ],
        sbr,
    );
    // right
    painter.line_segment(
        [
            Pos2::new(r.right() - h, r.top()),
            Pos2::new(r.right() - h, r.bottom()),
        ],
        sbr,
    );
}
