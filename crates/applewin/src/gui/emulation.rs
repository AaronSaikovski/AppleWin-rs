//! Emulator control: per-frame execution pacing, reset, disk (re)loading,
//! and slot-card configuration.

use super::*;

impl EmulatorApp {
    /// Reset the emulator and clear all audio state so no stale signal leaks through.
    pub(super) fn reset(&mut self, power_cycle: bool) {
        if let Some(ref mut iigs) = self.iigs {
            iigs.reset(power_cycle);
            self.last_audio_cycle = iigs.cpu.cycles;
        } else {
            self.emu
                .reset_with_pattern(power_cycle, self.config.memory_init_pattern);
            self.last_audio_cycle = self.emu.cpu.cycles;
        }
        // Silence the speaker: discard any pending toggles and reset the DC
        // filter so we don't output a fading ±0.5 hiss after the reset.
        self.speaker_state = false;
        self.dc_filter_ctr = 0;
        self.spkr_cycle_rem = 0.0;
        self.last_frame_time = std::time::Instant::now();
    }

    pub(super) fn reload_disk(
        emu: &mut Emulator,
        slot: usize,
        drive: usize,
        path: &Option<PathBuf>,
    ) {
        if let Some(p) = path {
            if let Ok(data) = std::fs::read(p) {
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                emu.bus.load_disk(slot, drive, &data, &ext);
                emu.bus.set_disk_path(slot, drive, p.clone());
            }
        } else {
            emu.bus.eject_disk(slot, drive);
        }
    }

    /// Load a SmartPort disk image into the IIgs emulator.
    /// Handles .2mg/.2img (with header) and .po/.hdv (raw ProDOS order).
    /// Returns true on success.
    pub(super) fn load_iigs_disk(
        iigs: &mut apple2_iigs::emulator::IIgsEmulator,
        drive: usize,
        path: &std::path::Path,
    ) -> bool {
        use apple2_iigs::smartport::SmartPortDisk;
        let Ok(data) = std::fs::read(path) else {
            eprintln!("Failed to read disk image: {}", path.display());
            return false;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let path_str = path.to_string_lossy().to_string();
        let disk = match ext.as_str() {
            "2mg" | "2img" => SmartPortDisk::from_2mg(&data, Some(path_str)),
            "po" | "hdv" => Some(SmartPortDisk::from_raw(data, Some(path_str))),
            _ => {
                // Try raw first (many .dsk files are actually ProDOS order)
                if data.len() % 512 == 0 {
                    Some(SmartPortDisk::from_raw(data, Some(path_str)))
                } else {
                    None
                }
            }
        };
        match disk {
            Some(d) => {
                iigs.bus.smartport.insert(drive, d);
                true
            }
            None => {
                eprintln!(
                    "Unsupported disk format for IIgs SmartPort: {}",
                    path.display()
                );
                false
            }
        }
    }

    pub(super) fn disk_display_name(path: &Option<PathBuf>) -> &str {
        path.as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("(empty)")
    }

    pub(super) fn run_emulation(&mut self, in_logo_mode: bool) {
        // ── Run emulator for elapsed wall-clock time (not during logo) ───
        //
        // We execute cycles proportional to real time elapsed since the last
        // update() call rather than a fixed "one frame worth" of cycles.
        // This decouples emulator speed from repaint frequency: window resize,
        // settings dialogs, and OS events all cause extra egui repaints, which
        // would otherwise run the emulator at 2-3× speed and corrupt game state.
        //
        // Cap at 100 ms (≈6 frames) so that minimising the window or attaching
        // a debugger doesn't cause a huge burst of catch-up execution.
        // Capture wall-clock time once for the whole frame — avoids multiple
        // system calls across the logo/normal/full-speed branches.
        let frame_now = std::time::Instant::now();

        if !in_logo_mode {
            let elapsed_secs = frame_now
                .duration_since(self.last_frame_time)
                .as_secs_f64()
                .min(0.1); // 100 ms cap
            self.last_frame_time = frame_now;

            // Skip execution when the debugger has paused the CPU
            let debugger_paused = self.debugger.active;

            // CPU clock rate: emulation_speed * 102_300 Hz (1× = 1.023 MHz)
            let base_hz = self.config.emulation_speed.max(1) as f64 * 102_300.0;

            if debugger_paused {
                // Debugger is paused — do not execute any cycles
            } else if let Some(ref mut iigs) = self.iigs {
                // ── IIgs execution path ──────────────────────────────────
                // IIgs runs at 2.8 MHz fast or 1.023 MHz slow.
                // Use 2.8 MHz as the base for fast mode.
                let iigs_hz = if iigs.bus.mega2.is_fast_mode() {
                    2_800_000.0
                } else {
                    1_023_000.0
                };
                let hz = self.config.emulation_speed.max(1) as f64 * iigs_hz / 10.0;
                let cycles = (elapsed_secs * hz) as u64;
                if cycles > 0 {
                    iigs.execute(cycles);
                }
            } else if self.config.enhanced_disk_speed && self.emu.bus.disk_motor_on() {
                // Full-speed mode — matches AppleWin's g_bFullSpeed behaviour
                // (IsConditionForFullSpeed: motor on + enhanced disk enabled).
                //
                // Run at maximum host CPU speed until the disk motor turns off,
                // with a 100 ms real-time budget per frame to keep the UI
                // responsive (same as AppleWin's per-iteration frame budget).
                // This makes disk-heavy boots/loads that take seconds at 1× speed
                // complete in well under 1 second on a modern machine.
                const BUDGET: std::time::Duration = std::time::Duration::from_millis(100);
                const BATCH: u64 = 100_000; // batch size between motor & timer checks
                let full_start = std::time::Instant::now();
                while self.emu.bus.disk_motor_on() && full_start.elapsed() < BUDGET {
                    self.emu.execute(BATCH);
                }
                // Reset timer so the next frame doesn't try to "catch up" for
                // the real time spent in the full-speed loop.
                self.last_frame_time = std::time::Instant::now();
                // Discard all speaker toggles and stale audio samples accumulated
                // during the full-speed burst — they represent boot/loading sounds
                // that have already "happened" in emulated time and would otherwise
                // drain from the ring buffer as 2–3 s of audible delay.
                self.emu.bus.speaker_toggles.clear();
                self.last_audio_cycle = self.emu.cpu.cycles;
                self.dc_filter_ctr = 0;
                self.speaker_state = false;
                self.spkr_cycle_rem = 0.0;
                if let Some(buf) = &self.audio_buf {
                    buf.lock().unwrap().clear();
                }
            } else {
                let cycles = (elapsed_secs * base_hz) as u64;
                if cycles > 0 {
                    self.emu.execute(cycles);
                }
            }
        } else {
            // Keep last_frame_time current so we don't burst when logo exits.
            self.last_frame_time = frame_now;
        }
    }
}
// ── Slot helpers ──────────────────────────────────────────────────────────

/// Rebuild the emulator's card manager from the slot configuration.
///
/// Removes all existing cards then inserts new ones for each non-Empty slot
/// according to `config.slot_cards`.  Only card types with a live
/// implementation are actually inserted; unimplemented types are silently
/// skipped (the slot remains empty).
pub(super) fn apply_slot_cards(emu: &mut Emulator, config: &Config) {
    use apple2_core::cards::col80::{Col80Card, Extended80ColCard};
    use apple2_core::cards::disk2::Disk2Card;
    use apple2_core::cards::fourplay::FourPlayCard;
    use apple2_core::cards::hd::HdCard;
    use apple2_core::cards::languagecard::LanguageCardCard;
    use apple2_core::cards::megaaudio::MegaAudioCard;
    use apple2_core::cards::mockingboard::MockingboardCard;
    use apple2_core::cards::mouse::MouseCard;
    use apple2_core::cards::noslotclock::NoSlotClockCard;
    use apple2_core::cards::phasor::PhasorCard;
    use apple2_core::cards::printer::PrinterCard;
    use apple2_core::cards::ramworks::RamWorksCard;
    use apple2_core::cards::sam::SamCard;
    use apple2_core::cards::saturn::Saturn128KCard;
    use apple2_core::cards::sdmusic::SdMusicCard;
    use apple2_core::cards::snesmax::SnesMaxCard;
    use apple2_core::cards::ssc::SscCard;
    use apple2_core::cards::uthernet::UthernCard;
    use apple2_core::cards::vidhd::VidHdCard;
    use apple2_core::cards::z80card::Z80Card;
    // Clear all slots first
    for slot in 0..apple2_core::card::NUM_SLOTS {
        emu.bus.cards.remove(slot);
    }

    // Apple IIc: install built-in peripherals (no user-configurable slots).
    if emu.model.is_iic() {
        emu.bus.cards.insert(Box::new(SscCard::new(1))); // modem port
        emu.bus.cards.insert(Box::new(SscCard::new(2))); // printer port
        // Slot 3: 80-col handled by ROM + bus soft-switches
        emu.bus.cards.insert(Box::new(MouseCard::new(4)));
        // Slot 5: empty
        // The //c drives its disk port through the internal IWM firmware, so
        // enable IWM status-register semantics on the Disk II controller.
        let mut disk = Disk2Card::new(6);
        disk.set_iwm(true);
        emu.bus.cards.insert(Box::new(disk));
        // Slot 7: empty
        // No aux card — IIc has 128KB built-in (aux_ram is always present)
        return;
    }

    // Re-insert according to config
    for (slot, &card_type) in config.slot_cards.iter().enumerate() {
        match card_type {
            CardType::Disk2 => {
                emu.bus.cards.insert(Box::new(Disk2Card::new(slot)));
            }
            CardType::GenericHdd => {
                let mut card = HdCard::new(slot);
                // Auto-load HDD images from config
                if let Some(ref p) = config.last_hdd1
                    && let Ok(data) = std::fs::read(p)
                {
                    card.load_image(0, data);
                }
                if let Some(ref p) = config.last_hdd2
                    && let Ok(data) = std::fs::read(p)
                {
                    card.load_image(1, data);
                }
                emu.bus.cards.insert(Box::new(card));
            }
            CardType::Mockingboard => {
                emu.bus.cards.insert(Box::new(MockingboardCard::new(slot)));
            }
            CardType::MouseInterface => {
                emu.bus.cards.insert(Box::new(MouseCard::new(slot)));
            }
            CardType::Ssc => {
                emu.bus.cards.insert(Box::new(SscCard::new(slot)));
            }
            CardType::Phasor => {
                emu.bus.cards.insert(Box::new(PhasorCard::new(slot)));
            }
            CardType::Col80 => {
                emu.bus.cards.insert(Box::new(Col80Card::new(slot)));
            }
            CardType::Extended80Col => {
                emu.bus.cards.insert(Box::new(Extended80ColCard::new(slot)));
            }
            CardType::Sam => {
                emu.bus.cards.insert(Box::new(SamCard::new(slot)));
            }
            CardType::GenericClock => {
                emu.bus.cards.insert(Box::new(NoSlotClockCard::new(slot)));
            }
            CardType::FourPlay => {
                emu.bus.cards.insert(Box::new(FourPlayCard::new(slot)));
            }
            CardType::SnesMax => {
                emu.bus.cards.insert(Box::new(SnesMaxCard::new(slot)));
            }
            CardType::Saturn128K => {
                emu.bus.cards.insert(Box::new(Saturn128KCard::new(slot)));
            }
            CardType::RamWorksIII => {
                emu.bus.cards.insert(Box::new(RamWorksCard::new(slot)));
            }
            CardType::GenericPrinter => {
                emu.bus.cards.insert(Box::new(PrinterCard::new(slot)));
            }
            CardType::VidHD => {
                emu.bus.cards.insert(Box::new(VidHdCard::new(slot)));
            }
            CardType::Z80 => {
                emu.bus.cards.insert(Box::new(Z80Card::new(slot)));
            }
            CardType::Uthernet => {
                emu.bus
                    .cards
                    .insert(Box::new(UthernCard::new_uthernet1(slot)));
            }
            CardType::Uthernet2 => {
                emu.bus
                    .cards
                    .insert(Box::new(UthernCard::new_uthernet2(slot)));
            }
            CardType::LanguageCard => {
                emu.bus.cards.insert(Box::new(LanguageCardCard::new(slot)));
            }
            CardType::MegaAudio => {
                emu.bus.cards.insert(Box::new(MegaAudioCard::new(slot)));
            }
            CardType::SdMusic => {
                emu.bus.cards.insert(Box::new(SdMusicCard::new(slot)));
            }
            _ => {} // not yet implemented — leave slot empty
        }
    }
    // Aux slot (slot 8 / SLOT_AUX)
    match config.aux_slot_card {
        CardType::Extended80Col => {
            emu.bus.cards.insert_aux(Box::new(Extended80ColCard::new(
                apple2_core::card::SLOT_AUX,
            )));
        }
        CardType::Col80 => {
            emu.bus
                .cards
                .insert_aux(Box::new(Col80Card::new(apple2_core::card::SLOT_AUX)));
        }
        CardType::RamWorksIII => {
            emu.bus
                .cards
                .insert_aux(Box::new(RamWorksCard::new(apple2_core::card::SLOT_AUX)));
        }
        _ => {} // Empty or unsupported — leave aux slot empty
    }
}
