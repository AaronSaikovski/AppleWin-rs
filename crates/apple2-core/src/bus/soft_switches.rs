//! Soft-switch dispatch for the $C000–$C0FF I/O page: keyboard, memory-mode
//! switches, video switches, speaker/cassette, annunciators, paddles, the
//! //c latched VBL flag, and slot $C08x–$C0FF peripheral I/O.

use super::*;

impl Bus {
    // ── Soft-switch dispatch ($C000–$C0FF) ───────────────────────────────────

    pub(super) fn soft_switch_read(&mut self, reg: u8, cycles: u64) -> u8 {
        // $C000–$C0FF: Apple //e soft switches + slot peripheral I/O
        match reg {
            0x00 => self.keyboard_data,
            0x10 => {
                let old = self.keyboard_data;
                self.keyboard_data &= 0x7F;
                old
            }
            // $C028: ROMSWITCH — toggle ROM bank on Apple IIc (read-strobe).
            0x28 => {
                if self.model.is_iic() {
                    self.mode.toggle(MemMode::MF_ALTROM0);
                }
                self.floating_bus
            }
            0x30 => {
                self.speaker_state = !self.speaker_state;
                if self.speaker_toggles.len() < SPEAKER_TOGGLES_MAX {
                    self.speaker_toggles.push(cycles);
                }
                self.floating_bus
            }
            0x11 => self.flag_byte(MemMode::MF_BANK2),
            0x12 => self.flag_byte(MemMode::MF_HIGHRAM),
            0x13 => self.flag_byte(MemMode::MF_AUXREAD),
            0x14 => self.flag_byte(MemMode::MF_AUXWRITE),
            0x15 => self.flag_byte(MemMode::MF_INTCXROM),
            0x16 => self.flag_byte(MemMode::MF_ALTZP),
            0x17 => self.flag_byte(MemMode::MF_SLOTC3ROM),
            0x18 => self.flag_byte(MemMode::MF_80STORE),
            // $C019: VBLANK.
            //
            // Apple //c: latched VBL interrupt-pending flag — bit 7 is set at the
            // start of each vertical blanking period and held until acknowledged
            // by an access to $C070.  Games frame-sync sound/music with
            // `LDA $C019 / BPL …` then `STA $C070`; returning the IIe's live
            // signal here makes those waits fall through instantly (music and
            // sound effects free-run into a screech).
            //
            // IIe: live VBL bar — bit 7 = 1 during visible scan lines, 0 in the
            // blanking interval.  NTSC: 192 active lines × 65 CPU cycles/line =
            // 12480; frame = 262 × 65 = 17030.  Matches AppleWin's
            // NTSC_GetVblBar(): true when g_nVideoClockVert < 192.
            //
            // We avoid the expensive modulo by tracking `frame_start_cycles` and computing
            // the within-frame offset as a simple subtraction.  The frame boundary is
            // advanced lazily here; `advance_frame()` may also be called from the execute loop.
            0x19 if self.model.is_iic() => {
                if self.vbl_flag {
                    0x80
                } else {
                    0x00
                }
            }
            0x19 => {
                let mut offset = cycles.wrapping_sub(self.frame_start_cycles);
                if offset >= CYCLES_PER_FRAME {
                    // Advance by whole frames so frame_start_cycles stays accurate even if
                    // advance_frame() was not called between frames.
                    let elapsed_frames = offset / CYCLES_PER_FRAME;
                    self.frame_start_cycles += elapsed_frames * CYCLES_PER_FRAME;
                    offset -= elapsed_frames * CYCLES_PER_FRAME;
                }
                if offset < CYCLES_VISIBLE { 0x80 } else { 0x00 }
            }
            // $C01A: RDTEXT — bit 7 = 1 when TEXT mode (graphics switch clear)
            0x1A => {
                if !self.mode.contains(MemMode::MF_GRAPHICS) {
                    0x80
                } else {
                    0x00
                }
            }
            // $C01B: RDMIXED — bit 7 = 1 when mixed mode
            0x1B => self.flag_byte(MemMode::MF_MIXED),
            0x1C => self.flag_byte(MemMode::MF_PAGE2),
            0x1D => self.flag_byte(MemMode::MF_HIRES),
            0x1E => self.flag_byte(MemMode::MF_ALTCHAR),
            0x1F => self.flag_byte(MemMode::MF_VID80),
            // $C061–$C063: game port buttons (bit 7 = pressed)
            0x61 => {
                if self.gamepad.effective_buttons() & 0x01 != 0 {
                    0x80
                } else {
                    0x00
                }
            }
            0x62 => {
                if self.gamepad.effective_buttons() & 0x02 != 0 {
                    0x80
                } else {
                    0x00
                }
            }
            0x63 => {
                if self.gamepad.effective_buttons() & 0x04 != 0 {
                    0x80
                } else {
                    0x00
                }
            }
            // $C064–$C067: paddle one-shot timers (bit 7 high until timer expires)
            0x64 => {
                if cycles < self.gamepad.paddle0_end {
                    0x80
                } else {
                    0x00
                }
            }
            0x65 => {
                if cycles < self.gamepad.paddle1_end {
                    0x80
                } else {
                    0x00
                }
            }
            0x66 | 0x67 => 0x00, // paddles 2/3 not connected
            // $C070: paddle strobe — resets timers and returns floating bus.
            // On the //c this also acknowledges (clears) the VBL flag.
            0x70 => {
                self.gamepad.strobe(cycles);
                if self.model.is_iic() {
                    self.vbl_flag = false;
                    self.update_irq_line();
                }
                self.floating_bus
            }
            // $C050–$C057: video soft-switch reads are strobes just like writes
            0x50 => {
                self.mode.insert(MemMode::MF_GRAPHICS);
                self.floating_bus
            }
            0x51 => {
                self.mode.remove(MemMode::MF_GRAPHICS);
                self.floating_bus
            }
            0x52 => {
                self.mode.remove(MemMode::MF_MIXED);
                self.floating_bus
            }
            0x53 => {
                self.mode.insert(MemMode::MF_MIXED);
                self.floating_bus
            }
            // $C054–$C057: PAGE2 / HIRES soft-switch reads act as strobes.
            // Only rebuild the page tables when the bit actually changes — programs
            // that poll these registers in tight loops would otherwise trigger a full
            // rebuild on every read even when the mode is unchanged.
            0x54 => {
                if self.mode.contains(MemMode::MF_PAGE2) {
                    self.mode.remove(MemMode::MF_PAGE2);
                    self.rebuild_page_tables();
                }
                self.floating_bus
            }
            0x55 => {
                if !self.mode.contains(MemMode::MF_PAGE2) {
                    self.mode.insert(MemMode::MF_PAGE2);
                    self.rebuild_page_tables();
                }
                self.floating_bus
            }
            0x56 => {
                if self.mode.contains(MemMode::MF_HIRES) {
                    self.mode.remove(MemMode::MF_HIRES);
                    self.rebuild_page_tables();
                }
                self.floating_bus
            }
            0x57 => {
                if !self.mode.contains(MemMode::MF_HIRES) {
                    self.mode.insert(MemMode::MF_HIRES);
                    self.rebuild_page_tables();
                }
                self.floating_bus
            }
            // $C058–$C05D: annunciators 0–2 (read-strobes, same as write)
            0x58 => {
                self.ann[0] = false;
                self.floating_bus
            }
            0x59 => {
                self.ann[0] = true;
                self.floating_bus
            }
            // $C05A/$C05B: annunciator 1 on the IIe; DISVBL/ENVBL on the //c
            // (mask for the VBL interrupt — the flag still latches either way).
            0x5A => {
                if self.model.is_iic() {
                    self.vbl_irq_enabled = false;
                    self.update_irq_line();
                } else {
                    self.ann[1] = false;
                }
                self.floating_bus
            }
            0x5B => {
                if self.model.is_iic() {
                    self.vbl_irq_enabled = true;
                    self.update_irq_line();
                } else {
                    self.ann[1] = true;
                }
                self.floating_bus
            }
            0x5C => {
                self.ann[2] = false;
                self.floating_bus
            }
            0x5D => {
                self.ann[2] = true;
                self.floating_bus
            }
            // $C05E/$C05F: DHIRESON/DHIRESOFF — read also acts as write (same as $C050-$C057)
            // On the IIc, $C05E/$C05F are the AN3 soft switch, which controls double
            // hi-res independently of IOUDIS (per the //c Technical Reference: double
            // hi-res operates regardless of the IOUDIS state).  So they toggle DHIRES
            // just like on the //e — software such as Airheart enables DHGR with a bare
            // $C05E without first setting IOUDIS.
            0x5E => {
                self.mode.insert(MemMode::MF_DHIRES);
                self.floating_bus
            }
            0x5F => {
                self.mode.remove(MemMode::MF_DHIRES);
                self.floating_bus
            }
            // $C060: cassette input — bit 7 reflects the cassette audio waveform.
            // When no cassette is loaded, returns 0 (high-impedance / silence).
            0x60 => {
                if let Some(ref data) = self.cassette_input {
                    // Derive sample position from CPU cycles elapsed since playback
                    // started.  Cassette audio is 11025 Hz; CPU is ~1.023 MHz.
                    // sample = (cycles - start) * 11025 / 1023000
                    const CASSETTE_RATE: u64 = 11025;
                    const CPU_RATE: u64 = 1_023_000;
                    let elapsed = cycles.saturating_sub(self.cassette_start_cycle);
                    let sample_pos = (elapsed * CASSETTE_RATE / CPU_RATE) as usize;
                    self.cassette_byte_pos = sample_pos;
                    if sample_pos < data.len() {
                        // Unsigned 8-bit PCM: 128 = silence.  Return bit 7 based
                        // on whether the sample is above or below the midpoint.
                        if data[sample_pos] >= 128 { 0x80 } else { 0x00 }
                    } else {
                        0x00 // past end of tape
                    }
                } else {
                    0x00
                }
            }
            // $C07E: RDIOUDES — bit 7 = 1 when IOUDIS is set; $C07D: alternate read
            0x7D | 0x7E => self.flag_byte(MemMode::MF_IOUDIS),
            // $C07F: RDDHIRES — bit 7 = 1 when double hi-res is active
            0x7F => self.flag_byte(MemMode::MF_DHIRES),
            0x80..=0x8F => self.lc_read(reg),
            // $C090–$C0FF: peripheral card I/O (slots 1–7)
            // $C09x = slot 1, $C0Ax = slot 2, ..., $C0Ex = slot 6, $C0Fx = slot 7
            0x90..=0xFF => {
                let slot = ((reg as usize) >> 4) - 8; // 0x90>>4=9 → slot 1 .. 0xF0>>4=15 → slot 7
                let lo = reg & 0x0F;
                if let Some(card) = self.cards.slot_mut(slot) {
                    let result = card.slot_io_read(lo, cycles);
                    self.process_card_dma(slot);
                    self.update_irq_line();
                    result
                } else {
                    self.floating_bus
                }
            }
            _ => self.floating_bus,
        }
    }

    pub(super) fn soft_switch_write(&mut self, reg: u8, val: u8, cycles: u64) {
        match reg {
            0x00 => {
                self.mode.remove(MemMode::MF_80STORE);
                self.rebuild_page_tables();
            }
            0x01 => {
                self.mode.insert(MemMode::MF_80STORE);
                self.rebuild_page_tables();
            }
            // $C010: KBDSTRB — writing clears the keyboard strobe (same as reading it).
            // Many programs use STA $C010 rather than LDA $C010 to clear the strobe.
            0x10 => {
                self.keyboard_data &= 0x7F;
            }
            0x02 => {
                self.mode.remove(MemMode::MF_AUXREAD);
                self.rebuild_page_tables();
            }
            0x03 => {
                self.mode.insert(MemMode::MF_AUXREAD);
                self.rebuild_page_tables();
            }
            0x04 => {
                self.mode.remove(MemMode::MF_AUXWRITE);
                self.rebuild_page_tables();
            }
            0x05 => {
                self.mode.insert(MemMode::MF_AUXWRITE);
                self.rebuild_page_tables();
            }
            0x06 if !self.model.is_iic() => {
                self.mode.remove(MemMode::MF_INTCXROM);
                self.rebuild_page_tables();
            }
            0x07 if !self.model.is_iic() => {
                self.mode.insert(MemMode::MF_INTCXROM);
                self.rebuild_page_tables();
            }
            0x08 => {
                self.mode.remove(MemMode::MF_ALTZP);
                self.rebuild_page_tables();
            }
            0x09 => {
                self.mode.insert(MemMode::MF_ALTZP);
                self.rebuild_page_tables();
            }
            0x0A if !self.model.is_iic() => {
                self.mode.remove(MemMode::MF_SLOTC3ROM);
                self.rebuild_page_tables();
            }
            0x0B if !self.model.is_iic() => {
                self.mode.insert(MemMode::MF_SLOTC3ROM);
                self.rebuild_page_tables();
            }
            // $C00C/$C00D: CLR/SET80VID — 80-column display mode
            0x0C => {
                self.mode.remove(MemMode::MF_VID80);
            }
            0x0D => {
                self.mode.insert(MemMode::MF_VID80);
            }
            // $C00E/$C00F: CLRALTCHAR/SETALTCHAR — alternate character set
            0x0E => {
                self.mode.remove(MemMode::MF_ALTCHAR);
            }
            0x0F => {
                self.mode.insert(MemMode::MF_ALTCHAR);
            }
            // $C028: ROMSWITCH — toggle ROM bank on Apple IIc.
            0x28 if self.model.is_iic() => {
                self.mode.toggle(MemMode::MF_ALTROM0);
            }
            // $C070: paddle strobe — reset one-shot timers.
            // On the //c this also acknowledges (clears) the VBL flag.
            0x70 => {
                self.gamepad.strobe(cycles);
                if self.model.is_iic() {
                    self.vbl_flag = false;
                    self.update_irq_line();
                }
            }
            // $C073: RamWorks III bank select
            0x73 => {
                self.rw3_switch(val);
            }
            0x30 => {
                self.speaker_state = !self.speaker_state;
                if self.speaker_toggles.len() < SPEAKER_TOGGLES_MAX {
                    self.speaker_toggles.push(cycles);
                }
            }
            // Text/graphics + mixed mode soft switches — video-only, no paging side-effects
            0x50 => {
                self.mode.insert(MemMode::MF_GRAPHICS);
            }
            0x51 => {
                self.mode.remove(MemMode::MF_GRAPHICS);
            }
            0x52 => {
                self.mode.remove(MemMode::MF_MIXED);
            }
            0x53 => {
                self.mode.insert(MemMode::MF_MIXED);
            }
            0x54 if self.mode.contains(MemMode::MF_PAGE2) => {
                self.mode.remove(MemMode::MF_PAGE2);
                self.rebuild_page_tables();
            }
            0x55 if !self.mode.contains(MemMode::MF_PAGE2) => {
                self.mode.insert(MemMode::MF_PAGE2);
                self.rebuild_page_tables();
            }
            0x56 if self.mode.contains(MemMode::MF_HIRES) => {
                self.mode.remove(MemMode::MF_HIRES);
                self.rebuild_page_tables();
            }
            0x57 if !self.mode.contains(MemMode::MF_HIRES) => {
                self.mode.insert(MemMode::MF_HIRES);
                self.rebuild_page_tables();
            }
            // $C058–$C05D: annunciators 0–2
            0x58 => {
                self.ann[0] = false;
            }
            0x59 => {
                self.ann[0] = true;
            }
            // $C05A/$C05B: annunciator 1 on the IIe; DISVBL/ENVBL on the //c.
            0x5A => {
                if self.model.is_iic() {
                    self.vbl_irq_enabled = false;
                    self.update_irq_line();
                } else {
                    self.ann[1] = false;
                }
            }
            0x5B => {
                if self.model.is_iic() {
                    self.vbl_irq_enabled = true;
                    self.update_irq_line();
                } else {
                    self.ann[1] = true;
                }
            }
            0x5C => {
                self.ann[2] = false;
            }
            0x5D => {
                self.ann[2] = true;
            }
            // $C05E/$C05F: DHIRESON/DHIRESOFF (AN3).  On the IIc this controls double
            // hi-res independently of IOUDIS, the same as on the //e (see read path).
            0x5E => {
                self.mode.insert(MemMode::MF_DHIRES);
            }
            0x5F => {
                self.mode.remove(MemMode::MF_DHIRES);
            }
            // $C07E: IOUDIS on; $C07F: IOUDIS off (in addition to DHIRESOFF read)
            0x7E => {
                self.mode.insert(MemMode::MF_IOUDIS);
            }
            0x7F => {
                self.mode.remove(MemMode::MF_IOUDIS);
            }
            0x80..=0x8F => self.lc_write(reg),
            0x90..=0xFF => {
                let slot = ((reg as usize) >> 4) - 8;
                let lo = reg & 0x0F;
                if let Some(card) = self.cards.slot_mut(slot) {
                    card.slot_io_write(lo, val, cycles);
                    self.process_card_dma(slot);
                    self.process_lc_bank_swap(slot);
                    self.update_irq_line();
                }
            }
            _ => {}
        }
    }

    #[inline]
    fn flag_byte(&self, flag: MemMode) -> u8 {
        if self.mode.contains(flag) { 0x80 } else { 0x00 }
    }

    /// Recompute `irq_line` from all IRQ sources: expansion cards, plus the
    /// //c VBL interrupt when enabled via ENVBL ($C05B).
    #[inline]
    pub(super) fn update_irq_line(&mut self) {
        self.irq_line = self.cards.any_irq_active() || (self.vbl_irq_enabled && self.vbl_flag);
    }

    /// Drain any pending DMA requests from a card and apply them to RAM.
    #[inline]
    fn process_card_dma(&mut self, slot: usize) {
        // DMA write: card → main RAM
        if let Some(card) = self.cards.slot_mut(slot)
            && let Some(DmaWrite { dest, data }) = card.take_dma_write()
        {
            let dest = dest as usize;
            let end = (dest + data.len()).min(65536);
            let len = end - dest;
            self.main_ram[dest..end].copy_from_slice(&data[..len]);
        }
        // DMA read: main RAM → card (pass slice directly; no heap copy needed)
        if let Some(card) = self.cards.slot_mut(slot)
            && let Some((src, len)) = card.take_dma_read_request()
        {
            let src = src as usize;
            let len = len as usize;
            let end = (src + len).min(65536);
            card.dma_read_complete(&self.main_ram[src..end]);
        }
    }
}
