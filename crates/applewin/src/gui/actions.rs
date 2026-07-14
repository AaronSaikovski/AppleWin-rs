//! Deferred UI actions: the [`DeferredActions`] request struct that panel
//! closures populate during layout, and [`EmulatorApp::apply_deferred_actions`]
//! which applies them once all panels have been drawn (so closures never need
//! `&mut self` twice).

use super::*;

/// UI actions requested by panel closures during a frame, applied after
/// all panels are laid out (so closures never need `&mut self` twice).
#[derive(Default)]
pub(super) struct DeferredActions {
    pub(super) hard_reset: bool,
    pub(super) reset: bool,
    pub(super) quit: bool,
    pub(super) load_disk1: bool,
    pub(super) load_disk2: bool,
    pub(super) eject_disk1: bool,
    pub(super) eject_disk2: bool,
    pub(super) swap: bool,
    pub(super) fullscreen: bool,
    pub(super) about: bool,
    pub(super) show_settings: bool,
    pub(super) screenshot: bool,
    pub(super) load_hdd1: bool,
    pub(super) load_hdd2: bool,
    pub(super) eject_hdd1: bool,
    pub(super) eject_hdd2: bool,
    pub(super) recent_disk: Option<String>,
    pub(super) recent_hdd: Option<String>,
}

impl EmulatorApp {
    pub(super) fn apply_deferred_actions(
        &mut self,
        ctx: &egui::Context,
        act: DeferredActions,
        in_logo_mode: bool,
    ) {
        // ── Apply deferred actions ────────────────────────────────────────
        if act.hard_reset {
            if self.config.confirm_reboot {
                self.pending_reset = Some(true);
            } else {
                self.reset(true);
            }
        }
        if act.reset {
            if self.config.confirm_reboot {
                self.pending_reset = Some(false);
            } else {
                self.reset(false);
            }
        }
        if act.quit {
            self.config.save();
            request_close(ctx);
        }
        if act.about {
            self.show_about = true;
        }
        if act.show_settings {
            self.pending_config = self.config.clone();
            self.show_settings = true;
        }
        if act.fullscreen {
            self.toggle_fullscreen(ctx);
        }
        if act.swap {
            // Swap path names
            std::mem::swap(&mut self.disk1, &mut self.disk2);
            std::mem::swap(&mut self.config.last_disk1, &mut self.config.last_disk2);
            // Re-load both drives so the card reflects the swap
            let d1 = self.disk1.clone();
            let d2 = self.disk2.clone();
            Self::reload_disk(&mut self.emu, self.disk_slot, 0, &d1);
            Self::reload_disk(&mut self.emu, self.disk_slot, 1, &d2);
        }
        if act.eject_disk1 {
            self.emu.bus.eject_disk(self.disk_slot, 0);
            self.disk1 = None;
            self.config.last_disk1 = None;
            self.config.save();
        }
        if act.eject_disk2 {
            self.emu.bus.eject_disk(self.disk_slot, 1);
            self.disk2 = None;
            self.config.last_disk2 = None;
            self.config.save();
        }
        if act.load_disk1 {
            let start_dir = self.config.last_disk_dir.as_deref();
            if let Some(path) = open_disk_dialog("Load Disk 1", start_dir) {
                let loaded = if let Some(ref mut iigs) = self.iigs {
                    Self::load_iigs_disk(iigs, 0, &path)
                } else if let Ok(data) = std::fs::read(&path) {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    self.emu.bus.load_disk(self.disk_slot, 0, &data, &ext);
                    true
                } else {
                    false
                };
                if loaded {
                    let path_str = path.to_string_lossy().into_owned();
                    self.config.add_recent_disk(&path_str);
                    self.config.last_disk1 = Some(path_str);
                    self.config.last_disk_dir =
                        path.parent().map(|p| p.to_string_lossy().into_owned());
                    self.disk1 = Some(path);
                    self.config.save();
                }
            }
        }
        if act.load_disk2 {
            let start_dir = self.config.last_disk_dir.as_deref();
            if let Some(path) = open_disk_dialog("Load Disk 2", start_dir) {
                let loaded = if let Some(ref mut iigs) = self.iigs {
                    Self::load_iigs_disk(iigs, 1, &path)
                } else if let Ok(data) = std::fs::read(&path) {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    self.emu.bus.load_disk(self.disk_slot, 1, &data, &ext);
                    true
                } else {
                    false
                };
                if loaded {
                    let path_str = path.to_string_lossy().into_owned();
                    self.config.add_recent_disk(&path_str);
                    self.config.last_disk2 = Some(path_str);
                    self.config.last_disk_dir =
                        path.parent().map(|p| p.to_string_lossy().into_owned());
                    self.disk2 = Some(path);
                    self.config.save();
                }
            }
        }
        // Load disk from recent list into drive 1
        if let Some(path_str) = act.recent_disk {
            let path = PathBuf::from(&path_str);
            let loaded = if let Some(ref mut iigs) = self.iigs {
                Self::load_iigs_disk(iigs, 0, &path)
            } else if let Ok(data) = std::fs::read(&path) {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                self.emu.bus.load_disk(self.disk_slot, 0, &data, &ext);
                self.emu.bus.set_disk_path(self.disk_slot, 0, path.clone());
                true
            } else {
                false
            };
            if loaded {
                self.config.add_recent_disk(&path_str);
                self.config.last_disk1 = Some(path_str);
                self.config.last_disk_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
                self.disk1 = Some(path);
                self.config.save();
            }
        }
        // HDD: load / eject
        if act.load_hdd1 {
            let start_dir = self.config.last_hdd_dir.as_deref();
            if let Some(path) = open_hdd_dialog("Load HDD 1", start_dir)
                && let Ok(data) = std::fs::read(&path)
            {
                let path_str = path.to_string_lossy().into_owned();
                self.config.add_recent_hdd(&path_str);
                self.config.last_hdd1 = Some(path_str);
                self.config.last_hdd_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
                // Apply to any installed HD card
                for slot in 0..apple2_core::card::NUM_SLOTS {
                    if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                        && card.card_type() == apple2_core::card::CardType::GenericHdd
                    {
                        if let Some(hd) = card
                            .as_any_mut()
                            .downcast_mut::<apple2_core::cards::hd::HdCard>()
                        {
                            hd.load_image(0, data);
                        }
                        break;
                    }
                }
                self.config.save();
            }
        }
        if act.load_hdd2 {
            let start_dir = self.config.last_hdd_dir.as_deref();
            if let Some(path) = open_hdd_dialog("Load HDD 2", start_dir)
                && let Ok(data) = std::fs::read(&path)
            {
                let path_str = path.to_string_lossy().into_owned();
                self.config.add_recent_hdd(&path_str);
                self.config.last_hdd2 = Some(path_str);
                self.config.last_hdd_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
                for slot in 0..apple2_core::card::NUM_SLOTS {
                    if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                        && card.card_type() == apple2_core::card::CardType::GenericHdd
                    {
                        if let Some(hd) = card
                            .as_any_mut()
                            .downcast_mut::<apple2_core::cards::hd::HdCard>()
                        {
                            hd.load_image(1, data);
                        }
                        break;
                    }
                }
                self.config.save();
            }
        }
        if act.eject_hdd1 {
            self.config.last_hdd1 = None;
            self.config.save();
        }
        if act.eject_hdd2 {
            self.config.last_hdd2 = None;
            self.config.save();
        }
        if let Some(path_str) = act.recent_hdd {
            let path = PathBuf::from(&path_str);
            if let Ok(data) = std::fs::read(&path) {
                self.config.add_recent_hdd(&path_str);
                self.config.last_hdd1 = Some(path_str);
                self.config.last_hdd_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
                for slot in 0..apple2_core::card::NUM_SLOTS {
                    if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                        && card.card_type() == apple2_core::card::CardType::GenericHdd
                    {
                        if let Some(hd) = card
                            .as_any_mut()
                            .downcast_mut::<apple2_core::cards::hd::HdCard>()
                        {
                            hd.load_image(0, data);
                        }
                        break;
                    }
                }
                self.config.save();
            }
        }

        // Screenshot from menu or F12 key
        if act.screenshot && !in_logo_mode {
            self.render_apple2();
            save_screenshot(self.fb.pixels_as_bytes(), SCREEN_W, SCREEN_H);
        }
    }
}
