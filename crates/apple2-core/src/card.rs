//! Card trait and CardManager.
//!
//! Replaces the `Card` base class + `CardManager` from `source/Card.h` and
//! `source/CardManager.h`.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

// ── Card type enum ─────────────────────────────────────────────────────────────

/// All known card types.  Values match the C++ `SS_CARDTYPE` enum so that
/// save-state files remain compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum CardType {
    Empty = 0,
    Disk2 = 1,
    Ssc = 2, // Super Serial Card
    Mockingboard = 3,
    GenericPrinter = 4,
    GenericHdd = 5,
    GenericClock = 6,
    MouseInterface = 7,
    Z80 = 8,
    Phasor = 9,
    Echo = 10,
    Sam = 11, // Software Automated Mouth
    Col80 = 12,
    Extended80Col = 13,
    RamWorksIII = 14,
    Uthernet = 15,
    LanguageCard = 16,
    LanguageCardIIe = 17,
    Saturn128K = 18,
    FourPlay = 19,
    SnesMax = 20,
    VidHD = 21,
    Uthernet2 = 22,
    MegaAudio = 23,
    SdMusic = 24,
    BreakpointCard = 25,
}

// ── Slot indices ──────────────────────────────────────────────────────────────

/// Number of regular slots (0–7).
pub const NUM_SLOTS: usize = 8;
/// Auxiliary slot index (//e extended memory etc.).
pub const SLOT_AUX: usize = 8;

// ── Card trait ─────────────────────────────────────────────────────────────────

// ── DMA support ───────────────────────────────────────────────────────────────

/// A pending DMA write from a card into Apple II main RAM.
pub struct DmaWrite {
    /// Destination address in Apple II main RAM.
    pub dest: u16,
    /// Bytes to write.
    pub data: Vec<u8>,
}

// ── Card trait ─────────────────────────────────────────────────────────────────

/// A plugged-in expansion card.
///
/// Each slot in the machine holds exactly one implementor of this trait.
/// The trait is object-safe so `Box<dyn Card>` can be stored in `CardManager`.
pub trait Card: Send + 'static {
    /// The type code for this card.
    fn card_type(&self) -> CardType;

    /// The slot number (0–7, or `SLOT_AUX`).
    fn slot(&self) -> usize;

    /// Read from the card's $Cnxx ROM page (offset 0x00–0xFF).
    /// Called when INTCXROM is off and the CPU reads from $Cnxx for this slot.
    fn io_read(&mut self, offset: u8, cycles: u64) -> u8;

    /// Write to the card's $Cnxx ROM page (rarely needed; mostly inhibited).
    fn io_write(&mut self, offset: u8, value: u8, cycles: u64);

    /// Return the card's $Cn00 ROM page as a static 256-byte slice, if any.
    fn cx_rom(&self) -> Option<&[u8; 256]> {
        None
    }

    /// Read from the card's peripheral I/O space ($C0x0–$C0xF, where x = slot+8).
    /// `reg` is the low nibble of the address (0x00–0x0F).
    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        let _ = reg;
        0xFF
    }

    /// Write to the card's peripheral I/O space.
    fn slot_io_write(&mut self, reg: u8, value: u8, _cycles: u64) {
        let _ = (reg, value);
    }

    /// Reset the card (power cycle if `power_cycle` is true, else warm reset).
    fn reset(&mut self, power_cycle: bool);

    /// Periodic update — called once per emulator quantum (~1ms / ~17030 cycles).
    fn update(&mut self, cycles: u64);

    /// Serialize state to a writer.
    fn save_state(&self, out: &mut dyn Write) -> Result<()>;

    /// Deserialize state from a reader.
    fn load_state(&mut self, src: &mut dyn Read, version: u32) -> Result<()>;

    /// Returns true if this card currently has a disk motor spinning.
    /// Only meaningful for Disk II cards; all others return false.
    fn disk_motor_on(&self) -> bool {
        false
    }

    /// Returns the activity state for the given drive (0 or 1).
    /// Only meaningful for Disk II cards; all others return the default (inactive).
    fn disk_drive_activity(&self, _drive: usize) -> DriveActivity {
        DriveActivity::default()
    }

    /// Optional DMA write to Apple II main RAM, triggered by a card I/O access.
    /// The Bus drains this after each slot_io_read or slot_io_write call.
    fn take_dma_write(&mut self) -> Option<DmaWrite> {
        None
    }

    /// Optional DMA read: card requests a slice of main RAM (e.g. for HD write).
    /// Returns `Some((src_addr, len))` if the card needs the bus to provide RAM data.
    fn take_dma_read_request(&mut self) -> Option<(u16, u16)> {
        None
    }

    /// Called by the Bus after fulfilling a DMA read request with the RAM slice.
    fn dma_read_complete(&mut self, _data: &[u8]) {}

    /// Drain accumulated audio samples into `out` (appending).
    /// Cards that produce audio (Mockingboard, Phasor) override this.
    /// `sample_rate` is the host audio output rate in Hz.
    /// The default no-op implementation is used by all non-audio cards.
    fn fill_audio(&mut self, _out: &mut Vec<f32>, _cycles_elapsed: u64, _sample_rate: u32) {}

    /// Update the card's mouse state (position + buttons).
    /// Only meaningful for mouse interface cards; others ignore this.
    fn set_mouse_state(&mut self, _x: i16, _y: i16, _buttons: u8) {}

    /// Take a pending language card bank swap.
    /// Returns `Some(new 16K bank data)` if this card wants to swap the LC area
    /// ($C000–$FFFF, 16384 bytes) in aux_ram.  The bus writes the new data into
    /// aux_ram and then calls `store_lc_bank` with the displaced old data.
    fn take_lc_bank_swap(&mut self) -> Option<Box<[u8; 16384]>> {
        None
    }

    /// Called by the bus to deliver the displaced LC bank data back to the card.
    /// The card should copy what it needs — the buffer belongs to the bus.
    fn store_lc_bank(&mut self, _data: &[u8; 16384]) {}

    /// Returns true if this card is currently asserting an IRQ.
    /// Cards that generate interrupts (SSC, Uthernet, etc.) should override this.
    fn irq_active(&self) -> bool {
        false
    }

    /// Downcast support — return `self` as `&mut dyn Any`.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

// ── EmptyCard ─────────────────────────────────────────────────────────────────

/// An empty slot — does nothing.
pub struct EmptyCard {
    slot: usize,
}

impl EmptyCard {
    pub fn new(slot: usize) -> Self {
        Self { slot }
    }
}

impl Card for EmptyCard {
    fn card_type(&self) -> CardType {
        CardType::Empty
    }
    fn slot(&self) -> usize {
        self.slot
    }
    fn io_read(&mut self, _offset: u8, _cycles: u64) -> u8 {
        0xFF
    }
    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}
    fn reset(&mut self, _power_cycle: bool) {}
    fn update(&mut self, _cycles: u64) {}
    fn save_state(&self, _out: &mut dyn Write) -> Result<()> {
        Ok(())
    }
    fn load_state(&mut self, _src: &mut dyn Read, _version: u32) -> Result<()> {
        Ok(())
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── DriveActivity ─────────────────────────────────────────────────────────────

/// Per-drive activity state for disk controllers.
#[derive(Clone, Copy, Debug, Default)]
pub struct DriveActivity {
    pub motor_on: bool,
    pub writing: bool,
    /// Current track number (0-34 for floppy, block-based for HDD).
    pub track: i32,
}

// ── CardManager ───────────────────────────────────────────────────────────────

/// Owns the 8 expansion slots plus the aux slot.
///
/// Replaces `CardManager` from `source/CardManager.h`.
pub struct CardManager {
    slots: [Option<Box<dyn Card>>; NUM_SLOTS],
    aux: Option<Box<dyn Card>>,
}

impl CardManager {
    pub fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            aux: None,
        }
    }

    /// Install a card in a slot (0–7).  Returns the previous occupant.
    pub fn insert(&mut self, card: Box<dyn Card>) -> Option<Box<dyn Card>> {
        let slot = card.slot();
        assert!(slot < NUM_SLOTS, "slot out of range: {slot}");
        self.slots[slot].replace(card)
    }

    /// Install a card in the aux slot.
    pub fn insert_aux(&mut self, card: Box<dyn Card>) -> Option<Box<dyn Card>> {
        self.aux.replace(card)
    }

    /// Remove a card from a slot.
    pub fn remove(&mut self, slot: usize) -> Option<Box<dyn Card>> {
        assert!(slot < NUM_SLOTS);
        self.slots[slot].take()
    }

    /// Immutable access to a slot.
    #[inline]
    pub fn slot(&self, slot: usize) -> Option<&dyn Card> {
        // Explicit range check lets the branch predictor learn the common
        // in-range path and allows the indexing below to elide its own check.
        if slot < NUM_SLOTS {
            self.slots[slot].as_deref()
        } else {
            None
        }
    }

    /// Mutable access to a slot.
    #[inline]
    pub fn slot_mut(&mut self, slot: usize) -> Option<&mut dyn Card> {
        if slot < NUM_SLOTS {
            self.slots[slot].as_deref_mut()
        } else {
            None
        }
    }

    /// Mutable access to the aux slot.
    pub fn aux_mut(&mut self) -> Option<&mut dyn Card> {
        self.aux.as_deref_mut()
    }

    /// Reset all cards (e.g. on machine reset).
    pub fn reset_all(&mut self, power_cycle: bool) {
        for slot in self.slots.iter_mut().flatten() {
            slot.reset(power_cycle);
        }
        if let Some(aux) = self.aux.as_deref_mut() {
            aux.reset(power_cycle);
        }
    }

    /// Returns true if any card (slot 0–7 or aux) currently asserts an IRQ.
    pub fn any_irq_active(&self) -> bool {
        self.slots
            .iter()
            .filter_map(|s| s.as_deref())
            .any(|c| c.irq_active())
            || self.aux.as_deref().is_some_and(|c| c.irq_active())
    }

    /// Update all cards (called each execution quantum).
    pub fn update_all(&mut self, cycles: u64) {
        for slot in self.slots.iter_mut().flatten() {
            slot.update(cycles);
        }
        if let Some(aux) = self.aux.as_deref_mut() {
            aux.update(cycles);
        }
    }
}

impl Default for CardManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Phase 2.3: slot / slot_mut take an explicit `slot < NUM_SLOTS` check
    /// rather than `get(slot)?`.  Verify the out-of-range path still returns
    /// None instead of panicking on the inner index.
    #[test]
    fn slot_out_of_range_returns_none() {
        let cards = CardManager::new();
        assert!(cards.slot(NUM_SLOTS).is_none());
        assert!(cards.slot(NUM_SLOTS + 5).is_none());
        assert!(cards.slot(usize::MAX).is_none());
    }

    #[test]
    fn slot_mut_out_of_range_returns_none() {
        let mut cards = CardManager::new();
        assert!(cards.slot_mut(NUM_SLOTS).is_none());
        assert!(cards.slot_mut(NUM_SLOTS + 5).is_none());
        assert!(cards.slot_mut(usize::MAX).is_none());
    }

    #[test]
    fn slot_empty_in_range_returns_none() {
        let cards = CardManager::new();
        for slot in 0..NUM_SLOTS {
            assert!(cards.slot(slot).is_none(), "slot {slot} should start empty");
        }
    }
}
