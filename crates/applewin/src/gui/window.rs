//! Native-window / viewport helpers.
//!
//! Centralizes every interaction with eframe's viewport API — window size,
//! position, maximized state, fullscreen, and close. Keeping this surface in
//! one module means an eframe upgrade (the viewport API is the part that
//! churns most between versions) touches a single file, and it keeps the
//! per-frame `update()` loop focused on emulation and UI rather than
//! window-management plumbing.

use super::*;

/// Chrome overhead added around the emulator screen when sizing the native
/// window: the menu bar, status bar, right-hand button strip, and borders.
const W_OVERHEAD: f32 = 80.0;
const H_OVERHEAD: f32 = 80.0;

/// Build the initial [`egui::ViewportBuilder`] from the persisted window
/// config: restores the saved maximized state and, when not maximized, the
/// previous inner size and screen position.
pub(super) fn build_viewport(config: &Config) -> egui::ViewportBuilder {
    let mut viewport = egui::ViewportBuilder::default()
        .with_min_inner_size(egui::vec2(
            SCREEN_W as f32 + W_OVERHEAD,
            SCREEN_H as f32 + H_OVERHEAD,
        ))
        .with_maximized(config.window_maximized);

    if !config.window_maximized {
        viewport = viewport.with_inner_size(egui::vec2(
            SCREEN_W as f32 * 2.0 + W_OVERHEAD,
            SCREEN_H as f32 * 2.0 + H_OVERHEAD,
        ));
        if let (Some(x), Some(y)) = (config.window_x, config.window_y) {
            viewport = viewport.with_position(egui::Pos2::new(x as f32, y as f32));
        }
    }
    viewport
}

/// Whether the OS window is currently maximized.
pub(super) fn window_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Ask the windowing system to close the application window.
pub(super) fn request_close(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

impl EmulatorApp {
    /// Toggle fullscreen, keeping [`Self::fullscreen`] in sync with the
    /// viewport command that actually drives the window.
    pub(super) fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.fullscreen = !self.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
    }

    /// Persist the current window size/position into config so the next launch
    /// restores it.
    ///
    /// `window_scale` is deliberately *not* tracked here — it only changes via
    /// the Settings dialog, so the saved value always reflects a deliberate
    /// user choice rather than an accidental resize. Position is only saved
    /// when not maximized, since the maximized rect is the OS-managed
    /// full-screen area and isn't useful to restore.
    pub(super) fn track_window_state(&mut self, ctx: &egui::Context) {
        let (maximized, outer_rect) = ctx.input(|i| {
            (
                i.viewport().maximized.unwrap_or(false),
                i.viewport().outer_rect,
            )
        });
        self.config.window_maximized = maximized;
        if !maximized && let Some(rect) = outer_rect {
            self.config.window_x = Some(rect.min.x as i32);
            self.config.window_y = Some(rect.min.y as i32);
        }
    }

    /// Resize the native window to fit the current `window_scale`.
    ///
    /// No-op while maximized: in that state the OS controls the window size, so
    /// we leave it alone and the new scale takes effect once the user
    /// un-maximizes.
    pub(super) fn resize_to_scale(&self, ctx: &egui::Context) {
        if window_maximized(ctx) {
            return;
        }
        let s = self.config.window_scale.max(1) as f32;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            SCREEN_W as f32 * s + BEVEL * 2.0 + BTN_PANEL_W + 24.0,
            SCREEN_H as f32 * s + BEVEL * 2.0 + 80.0,
        )));
    }
}
