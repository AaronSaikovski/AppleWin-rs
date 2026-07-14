//! The central Apple II screen panel (with 3D bevel and logo overlay) and
//! drag-and-drop disk insertion.

use super::*;

impl EmulatorApp {
    pub(super) fn show_central_panel(
        &mut self,
        ctx: &egui::Context,
        in_logo_mode: bool,
        tex_id: Option<egui::TextureId>,
    ) {
        let debugger_fullscreen = self.show_debugger && self.debugger.active;

        // ── Central panel — Apple II screen / debugger display ─────────────
        let central_bg = if debugger_fullscreen {
            Color32::BLACK
        } else {
            WIN_FACE
        };
        let central_margin = if debugger_fullscreen { 0.0 } else { 8.0 };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(central_bg)
                    .inner_margin(egui::Margin::same(central_margin)),
            )
            .show(ctx, |ui| {
                let avail = ui.available_rect_before_wrap();
                let sw = SCREEN_W as f32;
                let sh = SCREEN_H as f32;
                if debugger_fullscreen {
                    // Debugger mode: fill area, no bevel — fractional scale + linear filter
                    let scale_w = avail.width() / sw;
                    let scale_h = avail.height() / sh;
                    let scale = scale_w.min(scale_h).max(1.0);
                    let disp_w = sw * scale;
                    let disp_h = sh * scale;
                    let ox = avail.left() + ((avail.width() - disp_w) / 2.0).max(0.0);
                    let oy = avail.top() + ((avail.height() - disp_h) / 2.0).max(0.0);
                    let screen = Rect::from_min_size(Pos2::new(ox, oy), Vec2::new(disp_w, disp_h));
                    if let Some(tid) = tex_id {
                        ui.painter().image(
                            tid,
                            screen,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    ui.allocate_rect(avail, Sense::hover());
                } else {
                    // Normal mode: bevel + centred screen — fractional scale + linear filter
                    let avail_w = avail.width() - BEVEL * 2.0;
                    let avail_h = avail.height() - BEVEL * 2.0;
                    let scale_w = avail_w / sw;
                    let scale_h = avail_h / sh;
                    let scale = scale_w.min(scale_h).max(1.0);
                    let outer_w = sw * scale + BEVEL * 2.0;
                    let outer_h = sh * scale + BEVEL * 2.0;

                    let ox = avail.left() + ((avail.width() - outer_w) / 2.0).max(0.0);
                    let oy = avail.top() + ((avail.height() - outer_h) / 2.0).max(0.0);

                    let outer = Rect::from_min_size(Pos2::new(ox, oy), Vec2::new(outer_w, outer_h));
                    let screen = outer.shrink(BEVEL);

                    let painter = ui.painter();
                    draw_sunken_bevel(painter, outer);

                    if let Some(tid) = tex_id {
                        painter.image(
                            tid,
                            screen,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }

                    // ── Logo mode overlay — version + prompt ──────────────────
                    if in_logo_mode {
                        let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
                        // Version string position mirrors the C++ DRAWVERSION macro:
                        // scale*540 x scale*358 relative to the 560×384 logo area.
                        let vx = screen.left() + screen.width() * (540.0 / 560.0);
                        let vy = screen.top() + screen.height() * (358.0 / 384.0);
                        let vfont = FontId::proportional(13.0);
                        // 3-layer rendering matching C++ hi-colour path:
                        // (+1,+1) dark shadow, (-1,-1) light highlight, (0,0) main purple
                        painter.text(
                            Pos2::new(vx + 1.0, vy + 1.0),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont.clone(),
                            Color32::from_rgb(0x30, 0x30, 0x70),
                        );
                        painter.text(
                            Pos2::new(vx - 1.0, vy - 1.0),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont.clone(),
                            Color32::from_rgb(0xC0, 0x70, 0xE0),
                        );
                        painter.text(
                            Pos2::new(vx, vy),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont,
                            Color32::from_rgb(0x70, 0x30, 0xE0),
                        );
                        // "Press any key" prompt at bottom-centre
                        let px = screen.center().x;
                        let py = screen.bottom() - 10.0;
                        let pfont = FontId::proportional(13.0);
                        painter.text(
                            Pos2::new(px + 1.0, py + 1.0),
                            egui::Align2::CENTER_BOTTOM,
                            "Press any key to start",
                            pfont.clone(),
                            Color32::BLACK,
                        );
                        painter.text(
                            Pos2::new(px, py),
                            egui::Align2::CENTER_BOTTOM,
                            "Press any key to start",
                            pfont,
                            Color32::WHITE,
                        );
                    }

                    ui.allocate_rect(outer, Sense::hover());
                } // end else (normal mode)
            });
    }

    pub(super) fn handle_drag_and_drop(&mut self, ctx: &egui::Context) {
        // ── Drag-and-drop disk insertion ─────────────────────────────
        // First dropped file → drive 1, second → drive 2.
        {
            let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
            let disk_exts = [
                "dsk", "do", "po", "nib", "nb2", "woz", "d13", "gz", "zip", "2mg", "2img",
            ];
            for (i, file) in dropped.iter().enumerate() {
                if let Some(ref path) = file.path {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if disk_exts.iter().any(|&e| e == ext) {
                        let drive = i.min(1); // 0 or 1
                        let loaded = if let Some(ref mut iigs) = self.iigs {
                            Self::load_iigs_disk(iigs, drive, path)
                        } else if let Ok(data) = std::fs::read(path) {
                            self.emu.bus.load_disk(self.disk_slot, drive, &data, &ext);
                            self.emu
                                .bus
                                .set_disk_path(self.disk_slot, drive, path.clone());
                            true
                        } else {
                            false
                        };
                        if loaded {
                            let path_str = path.to_string_lossy().into_owned();
                            self.config.add_recent_disk(&path_str);
                            if drive == 0 {
                                self.disk1 = Some(path.clone());
                                self.config.last_disk1 = Some(path_str);
                            } else {
                                self.disk2 = Some(path.clone());
                                self.config.last_disk2 = Some(path_str);
                            }
                            self.config.last_disk_dir =
                                path.parent().map(|p| p.to_string_lossy().into_owned());
                            self.config.save();
                        }
                    }
                }
            }
        }
    }
}
