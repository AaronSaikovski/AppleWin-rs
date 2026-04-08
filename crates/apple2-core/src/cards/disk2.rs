//! Disk II interface card — 16-sector, .dsk/.do/.po/.nib/.woz image support.
//!
//! Implements the Disk II controller as described in "Beneath Apple DOS"
//! and translated from `source/Disk.cpp` / `source/DiskImageHelper.cpp`.
//!
//! WOZ v1/v2 support provides true bit-level disk emulation with:
//! - Bit-accurate track data (not nibble-converted)
//! - Cycle-accurate bit timing (~4 CPU cycles per bit at 250 kbit/s)
//! - Weak/flux bit randomization for copy-protected disks
//! - Quarter-track positioning via TMAP

use std::io::{Read, Write};
use crate::card::{Card, CardType};
use crate::error::Result;

// ── Constants ─────────────────────────────────────────────────────────────────

const NUM_TRACKS:           usize = 35;
const SECTORS_PER_TRACK:    usize = 16;
const SECTOR_SIZE:          usize = 256;
const DSK_SIZE:             usize = NUM_TRACKS * SECTORS_PER_TRACK * SECTOR_SIZE; // 143 360
const NIB_TRACK_SIZE:       usize = 6656;
const NIB_SIZE:             usize = NUM_TRACKS * NIB_TRACK_SIZE;                  // 232 960
const NB2_TRACK_SIZE:       usize = 6384;
const NB2_SIZE:             usize = NUM_TRACKS * NB2_TRACK_SIZE;                  // 223 440

// 13-sector (DOS 3.2) constants
const SECTORS_PER_TRACK_13: usize = 13;
const D13_SIZE:             usize = NUM_TRACKS * SECTORS_PER_TRACK_13 * SECTOR_SIZE; // 116 480

/// DOS 3.3 physical→logical sector skew (ms_SectorNumber[1] from DiskImageHelper.cpp).
const DOS33_SKEW: [u8; 16] = [
    0x00, 0x07, 0x0E, 0x06, 0x0D, 0x05, 0x0C, 0x04,
    0x0B, 0x03, 0x0A, 0x02, 0x09, 0x01, 0x08, 0x0F,
];

/// ProDOS physical→logical sector skew (ms_SectorNumber[0] from DiskImageHelper.cpp).
const PRODOS_SKEW: [u8; 16] = [
    0x00, 0x08, 0x01, 0x09, 0x02, 0x0A, 0x03, 0x0B,
    0x04, 0x0C, 0x05, 0x0D, 0x06, 0x0E, 0x07, 0x0F,
];

/// 6+2 GCR translation table: 6-bit index → valid disk byte (ms_DiskByte[] in DiskImageHelper.cpp).
const DISK_BYTE: [u8; 64] = [
    0x96, 0x97, 0x9A, 0x9B, 0x9D, 0x9E, 0x9F, 0xA6,
    0xA7, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF, 0xB2, 0xB3,
    0xB4, 0xB5, 0xB6, 0xB7, 0xB9, 0xBA, 0xBB, 0xBC,
    0xBD, 0xBE, 0xBF, 0xCB, 0xCD, 0xCE, 0xCF, 0xD3,
    0xD6, 0xD7, 0xD9, 0xDA, 0xDB, 0xDC, 0xDD, 0xDE,
    0xDF, 0xE5, 0xE6, 0xE7, 0xE9, 0xEA, 0xEB, 0xEC,
    0xED, 0xEE, 0xEF, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6,
    0xF7, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, 0xFF,
];

/// 5+3 GCR translation table: 5-bit index (0–31) → valid disk byte for 13-sector format.
const TRANS_5_3: [u8; 32] = [
    0xAB, 0xAD, 0xAE, 0xAF, 0xB5, 0xB6, 0xB7, 0xBA,
    0xBB, 0xBD, 0xBE, 0xBF, 0xD6, 0xD7, 0xDA, 0xDB,
    0xDD, 0xDE, 0xDF, 0xEA, 0xEB, 0xED, 0xEE, 0xEF,
    0xF5, 0xF6, 0xF7, 0xFA, 0xFB, 0xFD, 0xFE, 0xFF,
];

// ── DiskFormat ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiskFormat { Dos33, ProDos, Nib, Woz, Dos32 }

// ── WOZ bit-level structures ─────────────────────────────────────────────────

/// A single track stored as a packed bitstream (MSB-first within each byte).
#[derive(Clone)]
struct WozTrack {
    /// Packed bit stream (MSB-first within each byte).
    bits: Vec<u8>,
    /// Number of valid bits in the stream.
    bit_count: u32,
    /// Whether this track contains weak/flux bit areas.
    has_weak_bits: bool,
    /// Bitmask: 1 = this bit position is a weak bit (randomized on read).
    /// Only allocated if `has_weak_bits` is true.
    weak_mask: Vec<u8>,
}

impl WozTrack {
    /// Read a single bit at position `pos`.  If it's a weak bit, randomize it.
    #[inline]
    fn read_bit(&self, pos: u32, rng: &mut SimpleRng) -> u8 {
        if pos >= self.bit_count { return 0; }
        let byte_idx = (pos / 8) as usize;
        let bit_idx = 7 - (pos % 8);

        // Check if this is a weak bit
        if self.has_weak_bits && byte_idx < self.weak_mask.len()
            && (self.weak_mask[byte_idx] >> bit_idx) & 1 != 0 {
            return rng.next_bit();
        }

        (self.bits[byte_idx] >> bit_idx) & 1
    }

    /// Write a single bit at position `pos`.
    #[inline]
    fn write_bit(&mut self, pos: u32, val: u8) {
        if pos >= self.bit_count { return; }
        let byte_idx = (pos / 8) as usize;
        let bit_idx = 7 - (pos % 8);
        if val != 0 {
            self.bits[byte_idx] |= 1 << bit_idx;
        } else {
            self.bits[byte_idx] &= !(1 << bit_idx);
        }
        // Clear weak bit status when overwritten
        if self.has_weak_bits && byte_idx < self.weak_mask.len() {
            self.weak_mask[byte_idx] &= !(1 << bit_idx);
        }
    }
}

/// A complete parsed WOZ disk image.
#[derive(Clone)]
#[allow(dead_code)]
struct WozImage {
    /// WOZ format version (1 or 2).
    version: u8,
    /// Tracks indexed by TRKS index (sparse — some may be None).
    tracks: Vec<Option<WozTrack>>,
    /// Quarter-track map: 160 entries mapping quarter-track 0–159 to a TRKS index,
    /// or 0xFF for empty.
    tmap: [u8; 160],
    /// Whether the disk is write-protected.
    write_protected: bool,
    /// Disk type: 1 = 5.25", 2 = 3.5"
    disk_type: u8,
    /// Whether tracks are synchronized.
    synchronized: bool,
}

impl WozImage {
    /// Look up the WozTrack for a given quarter-track index (0–159).
    fn track_for_quarter(&self, qt: usize) -> Option<&WozTrack> {
        if qt >= 160 { return None; }
        let idx = self.tmap[qt];
        if idx == 0xFF { return None; }
        self.tracks.get(idx as usize)?.as_ref()
    }

    /// Mutable track lookup for a given quarter-track.
    fn track_for_quarter_mut(&mut self, qt: usize) -> Option<&mut WozTrack> {
        if qt >= 160 { return None; }
        let idx = self.tmap[qt];
        if idx == 0xFF { return None; }
        self.tracks.get_mut(idx as usize)?.as_mut()
    }
}

/// Simple LFSR-based random number generator for weak bits.
/// We avoid depending on `rand` by using a 32-bit xorshift.
#[derive(Clone)]
struct SimpleRng {
    state: u32,
}

impl SimpleRng {
    fn new(seed: u32) -> Self {
        Self { state: if seed == 0 { 0xDEAD_BEEF } else { seed } }
    }

    #[inline]
    fn next(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }

    #[inline]
    fn next_bit(&mut self) -> u8 {
        (self.next() & 1) as u8
    }
}

// ── CRC32 (IEEE / zip) ──────────────────────────────────────────────────────

/// Compute CRC32 over a byte slice (standard IEEE polynomial).
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// CPU cycles between each bit at 250 kbit/s with a 1.023 MHz clock.
/// 1_023_000 / 250_000 = ~4.092 cycles per bit.
const CYCLES_PER_BIT: u64 = 4;

// ── Per-drive state ───────────────────────────────────────────────────────────

struct Drive {
    /// Pre-nibblized track data: 35 entries, each a GCR byte stream.
    /// Used for non-WOZ formats (DSK, PO, NIB, D13).
    tracks:          Vec<Vec<u8>>,
    /// Head position in quarter-tracks (0–159 for WOZ, 0–79 for nibble formats).
    phase:           i32,
    /// Cached integer track index (= phase / 2, clamped to 0..NUM_TRACKS-1).
    /// Used only for non-WOZ formats.
    current_track_idx: usize,
    /// Current byte offset within the current track buffer (nibble mode).
    byte_pos:        usize,
    /// Whether a disk image is loaded.
    loaded:          bool,
    /// Original raw image data (for write-back flush).
    raw:             Vec<u8>,
    /// Whether any write has been made to this drive since last flush.
    dirty:           bool,
    /// The disk image format (for write-back and WOZ).
    format:          DiskFormat,
    /// Path to the image file on disk (for write-back).
    path:            Option<std::path::PathBuf>,
    /// Whether this drive is write-protected.
    write_protected: bool,

    // ── WOZ bit-level fields ──────────────────────────────────────────────

    /// Parsed WOZ image data (None for non-WOZ formats).
    woz: Option<WozImage>,
    /// Current bit position within the current WOZ track.
    bit_pos: u32,
    /// Shift register for accumulating bits into a nibble.
    shift_reg: u8,
    /// Number of zero bits encountered consecutively (for MC3470 emulation).
    zero_count: u32,
    /// Last CPU cycle when a bit was read/written (for timing).
    last_cycle: u64,
    /// RNG state for weak/flux bits.
    rng: SimpleRng,
}

impl Drive {
    fn new() -> Self {
        Drive {
            tracks:            Vec::new(),
            phase:             0,
            current_track_idx: 0,
            byte_pos:          0,
            loaded:            false,
            raw:             Vec::new(),
            dirty:           false,
            format:          DiskFormat::Dos33,
            path:            None,
            write_protected: false,
            woz:             None,
            bit_pos:         0,
            shift_reg:       0,
            zero_count:      0,
            last_cycle:      0,
            rng:             SimpleRng::new(0xCAFE_BABE),
        }
    }

    /// Whether this drive is in WOZ bit-level mode.
    #[inline]
    fn is_woz(&self) -> bool {
        self.woz.is_some()
    }

    /// Recompute `current_track_idx` from `phase`.  Call after any write to `phase`.
    #[inline]
    fn update_track_idx(&mut self) {
        self.current_track_idx = (self.phase / 2).clamp(0, (NUM_TRACKS as i32) - 1) as usize;
    }

    /// Get the current quarter-track index for WOZ lookup.
    #[inline]
    fn quarter_track(&self) -> usize {
        self.phase.clamp(0, 159) as usize
    }

    // ── Nibble-mode read/write (non-WOZ) ─────────────────────────────────

    /// Return the next nibble and advance the byte pointer (nibble mode).
    fn read_nibble_legacy(&mut self) -> u8 {
        if !self.loaded { return 0xFF; }
        let buf = &self.tracks[self.current_track_idx];
        if buf.is_empty() { return 0xFF; }
        let n = buf[self.byte_pos];
        self.byte_pos = (self.byte_pos + 1) % buf.len();
        n
    }

    fn write_nibble_legacy(&mut self, byte: u8) {
        if self.write_protected || !self.loaded { return; }
        let t = self.current_track_idx;
        if t >= self.tracks.len() { return; }
        let buf = &mut self.tracks[t];
        if buf.is_empty() { return; }
        if self.byte_pos < buf.len() {
            buf[self.byte_pos] = byte;
        }
        self.byte_pos = (self.byte_pos + 1) % buf.len();
        self.dirty = true;
    }

    // ── WOZ bit-level read/write ─────────────────────────────────────────

    /// Advance the bit position by the number of bits that have elapsed since
    /// `last_cycle`, based on CPU cycle count.  Returns number of bits advanced.
    fn advance_bits(&mut self, cycles: u64) -> u32 {
        if cycles <= self.last_cycle { return 0; }
        let elapsed = cycles - self.last_cycle;
        let bits = (elapsed / CYCLES_PER_BIT) as u32;
        self.last_cycle = cycles;
        bits
    }

    /// Read a single bit from the current WOZ track at the current position.
    fn woz_read_bit(&mut self) -> u8 {
        let qt = self.quarter_track();
        let woz = self.woz.as_ref().unwrap();
        if let Some(track) = woz.track_for_quarter(qt) {
            let bit_count = track.bit_count;
            if bit_count == 0 { return 0; }
            let pos = self.bit_pos % bit_count;
            let bit = track.read_bit(pos, &mut self.rng);
            self.bit_pos = (pos + 1) % bit_count;
            bit
        } else {
            // No track data — return random bits (unformatted track)
            self.rng.next_bit()
        }
    }

    /// Read bits until a complete nibble is formed (1-bit followed by 7 more bits).
    /// This simulates the Apple II disk controller's shift register behavior.
    /// `cycles` is the current CPU cycle count for timing.
    ///
    /// Returns `Some(nibble)` when a full byte with bit 7 set is assembled,
    /// or `None` if the shift register hasn't accumulated a complete nibble yet.
    /// This matches real hardware: the data latch retains the previous value
    /// until a new valid nibble is ready.
    fn woz_read_nibble(&mut self, cycles: u64) -> Option<u8> {
        // Advance position based on elapsed cycles
        let bits_elapsed = self.advance_bits(cycles);

        // Simulate each bit that elapsed
        for _ in 0..bits_elapsed.min(128) {
            let bit = self.woz_read_bit();
            self.shift_reg = (self.shift_reg << 1) | bit;

            if self.shift_reg & 0x80 != 0 {
                // Full nibble accumulated
                let result = self.shift_reg;
                self.shift_reg = 0;
                return Some(result);
            }
        }

        // No complete nibble yet — caller should preserve the current latch value.
        None
    }

    /// Write a single bit to the current WOZ track at the current position.
    fn woz_write_bit(&mut self, val: u8) {
        let qt = self.quarter_track();
        let woz = self.woz.as_mut().unwrap();
        if let Some(track) = woz.track_for_quarter_mut(qt) {
            let bit_count = track.bit_count;
            if bit_count == 0 { return; }
            let pos = self.bit_pos % bit_count;
            track.write_bit(pos, val);
            self.bit_pos = (pos + 1) % bit_count;
            self.dirty = true;
        }
    }

    /// Write a full nibble (8 bits, MSB first) to the current WOZ track.
    fn woz_write_nibble(&mut self, byte: u8, cycles: u64) {
        if self.write_protected { return; }

        // Advance position based on elapsed cycles
        let bits_elapsed = self.advance_bits(cycles);

        // Skip bits that elapsed while not writing
        let qt = self.quarter_track();
        let woz = self.woz.as_ref().unwrap();
        if let Some(track) = woz.track_for_quarter(qt) {
            let bit_count = track.bit_count;
            if bit_count > 0 {
                self.bit_pos = (self.bit_pos + bits_elapsed) % bit_count;
            }
        }

        // Write the 8 bits MSB first
        for i in (0..8).rev() {
            let bit = (byte >> i) & 1;
            self.woz_write_bit(bit);
        }
    }

    // ── Unified read/write dispatchers ───────────────────────────────────

    /// Read next nibble from disk.  Returns `Some(nibble)` when data is
    /// available, or `None` when the WOZ shift register hasn't formed a
    /// complete byte yet (caller should preserve the current data latch).
    fn read_nibble(&mut self, cycles: u64) -> Option<u8> {
        if self.is_woz() {
            self.woz_read_nibble(cycles)
        } else {
            Some(self.read_nibble_legacy())
        }
    }

    fn write_nibble(&mut self, byte: u8, cycles: u64) {
        if self.is_woz() {
            self.woz_write_nibble(byte, cycles);
        } else {
            self.write_nibble_legacy(byte);
        }
    }

    fn flush(&mut self) {
        if !self.dirty { return; }
        self.dirty = false;
        let Some(ref path) = self.path else { return; };
        match self.format {
            DiskFormat::Nib => {
                // For NIB: reassemble the flat nibble image from tracks
                let mut data = vec![0xFFu8; NIB_SIZE];
                for (t, track) in self.tracks.iter().enumerate() {
                    let start = t * NIB_TRACK_SIZE;
                    let copy_len = track.len().min(NIB_TRACK_SIZE);
                    data[start..start + copy_len].copy_from_slice(&track[..copy_len]);
                }
                let _ = std::fs::write(path, &data);
            }
            DiskFormat::Dos33 => {
                let skew = &DOS33_SKEW;
                let mut raw = std::mem::take(&mut self.raw);
                if raw.len() < DSK_SIZE { raw.resize(DSK_SIZE, 0); }
                for t in 0..NUM_TRACKS {
                    denibblize_track(&self.tracks[t], t as u8, skew, &mut raw[t * SECTORS_PER_TRACK * SECTOR_SIZE..]);
                }
                let _ = std::fs::write(path, &raw);
                self.raw = raw;
            }
            DiskFormat::ProDos => {
                let skew = &PRODOS_SKEW;
                let mut raw = std::mem::take(&mut self.raw);
                if raw.len() < DSK_SIZE { raw.resize(DSK_SIZE, 0); }
                for t in 0..NUM_TRACKS {
                    denibblize_track(&self.tracks[t], t as u8, skew, &mut raw[t * SECTORS_PER_TRACK * SECTOR_SIZE..]);
                }
                let _ = std::fs::write(path, &raw);
                self.raw = raw;
            }
            DiskFormat::Woz => {
                // WOZ write-back: not yet supported (would need to repack bitstream)
                // Track data is modified in-memory; changes are lost on eject.
            }
            DiskFormat::Dos32 => {
                // 13-sector write-back not supported
            }
        }
    }
}

// ── Disk2Card ─────────────────────────────────────────────────────────────────

pub struct Disk2Card {
    slot:         usize,
    drives:       [Drive; 2],
    active_drive: usize,
    motor_on:     bool,
    write_mode:   bool,
    latch:        u8,
    /// Stepper magnet states, bits 0–3 = phases 0–3.
    phases:       u8,
}

impl Disk2Card {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            drives:       [Drive::new(), Drive::new()],
            active_drive: 0,
            motor_on:     false,
            write_mode:   false,
            latch:        0xFF,
            phases:       0,
        }
    }

    /// Returns true if the disk motor is currently spinning.
    pub fn motor_on(&self) -> bool {
        self.motor_on
    }

    /// Load a disk image into `drive` (0 or 1).
    ///
    /// `ext` is the lowercase file extension used to select sector ordering:
    /// `"dsk"` / `"do"` → DOS 3.3 order, `"po"` → ProDOS order,
    /// `"nib"` → raw nibbles, `"woz"` → WOZ bitstream (bit-level emulation).
    ///
    /// WOZ images are loaded in bit-level mode: the drive maintains a bit
    /// position and shift register, and the controller reads one bit per
    /// ~4 CPU cycles.  Non-WOZ images continue to use nibble-level mode.
    pub fn load_drive(&mut self, drive: usize, data: &[u8], ext: &str) -> bool {
        if drive >= 2 { return false; }

        // Auto-detect WOZ by magic bytes, regardless of extension
        let is_woz_magic = data.len() >= 8
            && (data.starts_with(b"WOZ1\xff\x0a\x0d\x0a")
             || data.starts_with(b"WOZ2\xff\x0a\x0d\x0a"));

        if ext == "woz" || is_woz_magic {
            // Load as WOZ bit-level image
            if let Some(woz) = parse_woz(data) {
                let wp = woz.write_protected;
                let d = &mut self.drives[drive];
                d.woz               = Some(woz);
                d.tracks            = Vec::new(); // not used in WOZ mode
                d.loaded            = true;
                d.byte_pos          = 0;
                d.bit_pos           = 0;
                d.shift_reg         = 0;
                d.zero_count        = 0;
                d.phase             = 0;
                d.current_track_idx = 0;
                d.dirty             = false;
                d.format            = DiskFormat::Woz;
                d.raw               = data.to_vec();
                d.write_protected   = wp;
                return true;
            }
            return false;
        }

        let (format, tracks, write_protected) = match ext {
            "po" => (DiskFormat::ProDos, nibblize_image(data, &PRODOS_SKEW), false),
            "nib" => (DiskFormat::Nib, load_nib(data), false),
            "nb2" => (DiskFormat::Nib, load_nib_sized(data, NB2_TRACK_SIZE), false),
            "d13" => (DiskFormat::Dos32, nibblize_image_13(data), false),
            _ => {
                // Auto-detect by file size.
                if data.len() == D13_SIZE {
                    (DiskFormat::Dos32, nibblize_image_13(data), false)
                } else if data.len() == NB2_SIZE {
                    // NB2: 6384-byte nibble tracks.
                    (DiskFormat::Nib, load_nib_sized(data, NB2_TRACK_SIZE), false)
                } else {
                    (DiskFormat::Dos33, nibblize_image(data, &DOS33_SKEW), false)
                }
            }
        };
        if let Some(t) = tracks {
            let d = &mut self.drives[drive];
            d.woz               = None; // ensure not in WOZ mode
            d.tracks            = t;
            d.loaded            = true;
            d.byte_pos          = 0;
            d.phase             = 0;
            d.current_track_idx = 0;
            d.dirty             = false;
            d.format            = format;
            d.raw               = data.to_vec();
            d.write_protected   = write_protected;
            true
        } else {
            false
        }
    }

    /// Set the file path for write-back on `drive` (0 or 1).
    pub fn set_drive_path(&mut self, drive: usize, path: std::path::PathBuf) {
        if drive < 2 {
            self.drives[drive].path = Some(path);
        }
    }

    pub fn eject_drive(&mut self, drive: usize) {
        if drive < 2 {
            self.drives[drive].flush();
            self.drives[drive].loaded = false;
            self.drives[drive].tracks.clear();
            self.drives[drive].raw.clear();
            self.drives[drive].path = None;
        }
    }

    /// Update the stepper motor phases and move the head if needed.
    fn step_phase(&mut self, reg: u8) {
        let phase     = (reg >> 1) & 3;
        let phase_bit = 1u8 << phase;

        if reg & 1 != 0 {
            self.phases |= phase_bit;
        } else {
            self.phases &= !phase_bit;
        }

        let drive = &mut self.drives[self.active_drive];
        let cur   = drive.phase;
        // Mirror C++ ControlStepperDeferred: additive, same as C++.
        // When both adjacent phases are active, +1 and -1 cancel → no movement.
        // This matches the Apple II hardware and avoids overshooting during RWTS seeks.
        let fwd = if self.phases & (1 << ((cur + 1) & 3)) != 0 {  1i32 } else { 0 };
        let bwd = if self.phases & (1 << ((cur + 3) & 3)) != 0 { -1i32 } else { 0 };
        drive.phase = (cur + fwd + bwd).clamp(0, 79);
        drive.update_track_idx();
    }
}

impl Card for Disk2Card {
    fn card_type(&self) -> CardType { CardType::Disk2 }
    fn slot(&self)       -> usize   { self.slot }

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        DISK2_FW[offset as usize]
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn cx_rom(&self) -> Option<&[u8; 256]> { Some(DISK2_FW) }

    fn slot_io_read(&mut self, reg: u8, cycles: u64) -> u8 {
        match reg {
            0x00..=0x07 => { self.step_phase(reg); self.latch }
            0x08 => {
                if self.motor_on {
                    self.drives[self.active_drive].flush();
                }
                self.motor_on = false;
                self.latch
            }
            0x09 => { self.motor_on = true;  self.latch }
            0x0A => { self.active_drive = 0; self.latch }
            0x0B => { self.active_drive = 1; self.latch }
            0x0C => {
                if self.write_mode && self.motor_on {
                    self.drives[self.active_drive].write_nibble(self.latch, cycles);
                } else if !self.write_mode && self.motor_on {
                    // Only update the data latch when a complete nibble is ready.
                    // Real hardware holds the previous latch value until the shift
                    // register produces a new byte with bit 7 set.
                    if let Some(nibble) = self.drives[self.active_drive].read_nibble(cycles) {
                        self.latch = nibble;
                    }
                }
                self.latch
            }
            0x0D => {
                if self.drives[self.active_drive].write_protected { 0x80 } else { 0x00 }
            }
            0x0E => { self.write_mode = false; self.latch }
            0x0F => { self.write_mode = true;  self.latch }
            _    => self.latch,
        }
    }

    fn slot_io_write(&mut self, reg: u8, value: u8, cycles: u64) {
        match reg {
            0x00..=0x07 => self.step_phase(reg),
            0x08 => {
                if self.motor_on {
                    self.drives[self.active_drive].flush();
                }
                self.motor_on = false;
            }
            0x09 => { self.motor_on = true; }
            0x0A => { self.active_drive = 0; }
            0x0B => { self.active_drive = 1; }
            0x0C => {
                // Q6L write: load latch (data to write on next strobe)
                if self.write_mode && self.motor_on {
                    self.latch = value;
                    self.drives[self.active_drive].write_nibble(value, cycles);
                }
            }
            0x0D => { self.latch = value; }        // Q6H: load data register
            0x0E => { self.write_mode = false; }   // Q7L: read mode
            0x0F => { self.write_mode = true; }    // Q7H: write mode
            _    => {}
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.motor_on     = false;
        self.write_mode   = false;
        self.latch        = 0xFF;
        self.phases       = 0;
        self.active_drive = 0;
        for d in &mut self.drives { d.byte_pos = 0; }
    }

    fn update(&mut self, _cycles: u64) {}

    fn save_state(&self, _out: &mut dyn Write) -> Result<()> { Ok(()) }
    fn load_state(&mut self, _src: &mut dyn Read, _version: u32) -> Result<()> { Ok(()) }

    fn disk_motor_on(&self) -> bool { self.motor_on }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

// ── GCR Nibblization ──────────────────────────────────────────────────────────

fn load_nib(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    load_nib_sized(data, NIB_TRACK_SIZE)
}

/// Load a nibble image with a configurable track size (6656 for .nib, 6384 for .nb2).
fn load_nib_sized(data: &[u8], track_size: usize) -> Option<Vec<Vec<u8>>> {
    let expected = NUM_TRACKS * track_size;
    if data.len() < expected { return None; }
    let mut tracks = Vec::with_capacity(NUM_TRACKS);
    for t in 0..NUM_TRACKS {
        let start = t * track_size;
        tracks.push(data[start..start + track_size].to_vec());
    }
    Some(tracks)
}

fn nibblize_image(data: &[u8], skew: &[u8; 16]) -> Option<Vec<Vec<u8>>> {
    if data.len() < DSK_SIZE { return None; }
    let mut tracks = Vec::with_capacity(NUM_TRACKS);
    for t in 0..NUM_TRACKS {
        let base = t * SECTORS_PER_TRACK * SECTOR_SIZE;
        tracks.push(nibblize_track(&data[base..], t as u8, skew));
    }
    Some(tracks)
}

fn nibblize_track(track_data: &[u8], track_num: u8, skew: &[u8; 16]) -> Vec<u8> {
    let mut nibs = Vec::with_capacity(6656);

    nibs.resize(48, 0xFF);

    for phys in 0u8..16 {
        let log     = skew[phys as usize] as usize;
        let off     = log * SECTOR_SIZE;
        let sector: [u8; 256] = track_data[off..off + SECTOR_SIZE]
            .try_into()
            .unwrap_or([0u8; 256]);

        let vol: u8 = 0xFE;
        let chk: u8 = vol ^ track_num ^ phys;

        nibs.extend_from_slice(&[0xD5, 0xAA, 0x96]);
        nibs.extend_from_slice(&[code44a(vol), code44b(vol)]);
        nibs.extend_from_slice(&[code44a(track_num), code44b(track_num)]);
        nibs.extend_from_slice(&[code44a(phys), code44b(phys)]);
        nibs.extend_from_slice(&[code44a(chk), code44b(chk)]);
        nibs.extend_from_slice(&[0xDE, 0xAA, 0xEB]);

        nibs.extend(std::iter::repeat_n(0xFF, 6));

        nibs.extend_from_slice(&[0xD5, 0xAA, 0xAD]);
        nibs.extend_from_slice(&code62(&sector));
        nibs.extend_from_slice(&[0xDE, 0xAA, 0xEB]);

        nibs.extend(std::iter::repeat_n(0xFF, 27));
    }

    nibs
}

fn nibblize_image_13(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    if data.len() < D13_SIZE { return None; }
    let mut tracks = Vec::with_capacity(NUM_TRACKS);
    for t in 0..NUM_TRACKS {
        let base = t * SECTORS_PER_TRACK_13 * SECTOR_SIZE;
        tracks.push(nibblize_track_13(&data[base..base + SECTORS_PER_TRACK_13 * SECTOR_SIZE], t as u8));
    }
    Some(tracks)
}

/// Nibblize a single track of 13-sector (DOS 3.2) data using 5+3 GCR encoding.
///
/// Track layout: 13 sectors × 256 bytes stored sequentially (no interleave).
/// Address field prologue: D5 AA B5; data field prologue: D5 AA AD.
fn nibblize_track_13(track_data: &[u8], track_num: u8) -> Vec<u8> {
    let mut nibs = Vec::with_capacity(6656);

    // Leading self-sync gap
    nibs.extend(std::iter::repeat_n(0xFF, 48));

    for phys in 0u8..(SECTORS_PER_TRACK_13 as u8) {
        let off = phys as usize * SECTOR_SIZE;
        let sector: [u8; 256] = track_data[off..off + SECTOR_SIZE]
            .try_into()
            .unwrap_or([0u8; 256]);

        let vol: u8 = 0xFE;
        let chk: u8 = vol ^ track_num ^ phys;

        // Address field: prologue D5 AA B5
        nibs.extend_from_slice(&[0xD5, 0xAA, 0xB5]);
        nibs.extend_from_slice(&[code44a(vol),       code44b(vol)]);
        nibs.extend_from_slice(&[code44a(track_num), code44b(track_num)]);
        nibs.extend_from_slice(&[code44a(phys),      code44b(phys)]);
        nibs.extend_from_slice(&[code44a(chk),       code44b(chk)]);
        nibs.extend_from_slice(&[0xDE, 0xAA, 0xEB]);

        // Self-sync gap between address and data fields
        nibs.extend(std::iter::repeat_n(0xFF, 6));

        // Data field: prologue D5 AA AD
        nibs.extend_from_slice(&[0xD5, 0xAA, 0xAD]);
        let (encoded, checksum) = code53(&sector);
        nibs.extend_from_slice(&encoded);
        nibs.push(checksum);
        nibs.extend_from_slice(&[0xDE, 0xAA, 0xEB]);

        // Inter-sector gap
        nibs.extend(std::iter::repeat_n(0xFF, 27));
    }

    nibs
}

/// Encode 256 sector bytes using 5+3 GCR for the 13-sector (DOS 3.2) format.
///
/// Structure (mirrors the 6+2 layout but uses 5-bit values and TRANS_5_3):
///   raw[0..86]   — secondary bytes: each packs the low 3 bits of three source bytes
///   raw[86..342] — primary bytes:   the high 5 bits of each of the 256 source bytes
///
/// The 342 raw 5-bit values are XOR-chained then translated through TRANS_5_3.
/// Returns (342 encoded bytes, checksum byte).
fn code53(sector: &[u8; 256]) -> ([u8; 342], u8) {
    let mut raw = [0u8; 342];

    // Secondary bytes: pack bits 1:0 of sector[i+172], bits 1:0 of sector[i+86],
    // and bit 0 of sector[i] into a 5-bit value (bits 4:3, 2:1, 0 respectively).
    for i in 0..86usize {
        let a = sector[i];
        let b = sector[i + 86];
        let c = if i + 172 < 256 { sector[i + 172] } else { 0 };
        raw[i] = ((c & 0x03) << 3) | ((b & 0x03) << 1) | (a & 0x01);
    }

    // Primary bytes: high 5 bits of each sector byte.
    for i in 0..256usize {
        raw[86 + i] = (sector[i] >> 3) & 0x1F;
    }

    // XOR-chain and translate through TRANS_5_3.
    let mut out = [0u8; 342];
    let mut prev: u8 = 0;
    for i in 0..342 {
        let cur = raw[i];
        out[i] = TRANS_5_3[((prev ^ cur) & 0x1F) as usize];
        prev = cur;
    }
    let checksum = TRANS_5_3[(prev & 0x1F) as usize];
    (out, checksum)
}

#[inline] fn code44a(a: u8) -> u8 { ((a >> 1) & 0x55) | 0xAA }
#[inline] fn code44b(a: u8) -> u8 { (a & 0x55) | 0xAA }

fn code62(sector: &[u8; 256]) -> [u8; 343] {
    let mut raw = [0u8; 342];
    let mut offset: u8 = 0xAC;
    let mut ri = 0usize;

    while offset != 0x02 {
        let a = sector[offset as usize]; offset = offset.wrapping_sub(0x56);
        let b = sector[offset as usize]; offset = offset.wrapping_sub(0x56);
        let c = sector[offset as usize]; offset = offset.wrapping_sub(0x53);

        let ra = (a & 1) << 1 | (a & 2) >> 1;
        let rb = (b & 1) << 1 | (b & 2) >> 1;
        let rc = (c & 1) << 1 | (c & 2) >> 1;
        raw[ri] = ((ra << 4) | (rb << 2) | rc) << 2;
        ri += 1;
    }
    raw[ri - 2] &= 0x3F;
    raw[ri - 1] &= 0x3F;

    raw[86..342].copy_from_slice(sector);

    let mut xored = [0u8; 343];
    let mut prev: u8 = 0;
    for i in 0..342 {
        xored[i] = prev ^ raw[i];
        prev = raw[i];
    }
    xored[342] = prev;

    let mut out = [0u8; 343];
    for i in 0..343 {
        out[i] = DISK_BYTE[(xored[i] >> 2) as usize];
    }
    out
}

// ── GCR De-nibblization (write-back) ─────────────────────────────────────────

/// Build inverse DISK_BYTE lookup: disk byte → 6-bit value, 0xFF for invalid.
fn disk_byte_inv() -> [u8; 256] {
    let mut inv = [0xFFu8; 256];
    for (idx, &b) in DISK_BYTE.iter().enumerate() {
        inv[b as usize] = idx as u8;
    }
    inv
}

/// Decode a 4-and-4 encoded byte pair back to the original byte.
#[inline]
fn decode44(a: u8, b: u8) -> u8 {
    ((a & 0x55) << 1) | (b & 0x55)
}

/// Decode 343 GCR disk bytes back to 256 sector bytes.
/// Returns None if checksum fails or an invalid GCR byte is encountered.
fn decode62(gcr: &[u8; 343], inv: &[u8; 256]) -> Option<[u8; 256]> {
    // Step 1: convert GCR bytes to 8-bit values (6-bit value in bits 7:2, bits 1:0 = 0)
    let mut vals = [0u8; 343];
    for i in 0..343 {
        let v = inv[gcr[i] as usize];
        if v == 0xFF { return None; }
        vals[i] = v << 2;
    }

    // Step 2: undo XOR chain
    let mut raw = [0u8; 342];
    raw[0] = vals[0];
    for i in 1..342 {
        raw[i] = vals[i] ^ raw[i - 1];
    }
    // vals[342] is the checksum = raw[341]
    if vals[342] != raw[341] { return None; }

    // Step 3: primary bytes are raw[86..342] (top 6 bits of each sector byte, in bits 7:2)
    let mut sector = [0u8; 256];
    for j in 0..256 {
        sector[j] = raw[86 + j] & 0xFC; // top 6 bits (bits 7:2), low 2 bits from secondary
    }

    // Step 4: apply secondary bytes (raw[0..86]) to restore bits 1:0
    // Each secondary byte has a 6-bit value in bits 7:2 of raw[ri]:
    //   bits 7:6 = ra (bits 0,1 of sector byte 'a', reversed: bit1 stored in pos0, bit0 in pos1)
    //   bits 5:4 = rb (same for 'b')
    //   bits 3:2 = rc (same for 'c')
    // The sector byte offsets follow the same traversal as code62 encode.
    let mut offset: u8 = 0xAC;
    for &raw_byte in raw.iter().take(86) {
        let s6 = raw_byte >> 2; // 6-bit secondary value
        let ra = (s6 >> 4) & 0x03;
        let rb = (s6 >> 2) & 0x03;
        let rc = s6 & 0x03;
        // Un-reverse: original bits 0,1 = reversed(ra) = swap bit0 and bit1
        let a_bits = ((ra & 1) << 1) | ((ra >> 1) & 1);
        let b_bits = ((rb & 1) << 1) | ((rb >> 1) & 1);
        let c_bits = ((rc & 1) << 1) | ((rc >> 1) & 1);

        let oa = offset as usize; offset = offset.wrapping_sub(0x56);
        let ob = offset as usize; offset = offset.wrapping_sub(0x56);
        let oc = offset as usize; offset = offset.wrapping_sub(0x53);

        if oa < 256 { sector[oa] |= a_bits; }
        if ob < 256 { sector[ob] |= b_bits; }
        if oc < 256 { sector[oc] |= c_bits; }
    }

    Some(sector)
}

/// Decode a nibble stream and write the 16 sectors into `out` (one track's worth of raw bytes).
/// `out` must be at least `SECTORS_PER_TRACK * SECTOR_SIZE` bytes.
fn denibblize_track(nibs: &[u8], track_num: u8, skew: &[u8; 16], out: &mut [u8]) {
    let inv = disk_byte_inv();
    let n = nibs.len();
    if n < 10 { return; }

    // Build a doubled buffer so we can scan across the wrap-around
    let mut buf = nibs.to_vec();
    buf.extend_from_slice(nibs);
    let buf_len = buf.len();

    let mut i = 0usize;
    let mut sectors_found = 0u32;

    while i < n && sectors_found < 16 {
        // Scan for address prologue D5 AA 96
        if buf[i] == 0xD5 && buf[i + 1] == 0xAA && buf[i + 2] == 0x96 {
            i += 3;
            if i + 8 > buf_len { break; }
            // Decode 4+4 encoded volume, track, sector, checksum
            let _vol = decode44(buf[i], buf[i + 1]); i += 2;
            let trk  = decode44(buf[i], buf[i + 1]); i += 2;
            let sec  = decode44(buf[i], buf[i + 1]); i += 2;
            let _chk = decode44(buf[i], buf[i + 1]); i += 2;

            if trk != track_num || sec >= 16 { continue; }

            // Scan for data prologue D5 AA AD (within next 100 bytes)
            let search_end = (i + 100).min(buf_len - 344);
            let mut found_data = false;
            while i < search_end {
                if buf[i] == 0xD5 && buf[i + 1] == 0xAA && buf[i + 2] == 0xAD {
                    i += 3;
                    found_data = true;
                    break;
                }
                i += 1;
            }
            if !found_data { continue; }

            // Read 343 GCR bytes
            if i + 343 > buf_len { break; }
            let gcr: [u8; 343] = buf[i..i + 343].try_into().unwrap();
            i += 343;

            // Decode GCR → sector bytes
            if let Some(sector) = decode62(&gcr, &inv) {
                // Map physical sector to logical sector via skew
                let log_sec = skew[sec as usize] as usize;
                let off = log_sec * SECTOR_SIZE;
                if off + SECTOR_SIZE <= out.len() {
                    out[off..off + SECTOR_SIZE].copy_from_slice(&sector);
                }
                sectors_found += 1;
            }
        } else {
            i += 1;
        }
    }
}

// ── WOZ format support (bit-level) ───────────────────────────────────────────

/// Parse a WOZ v1 or v2 disk image into a `WozImage` for bit-level emulation.
///
/// Validates the header magic and CRC32, then parses INFO, TMAP, and TRKS
/// chunks.  For WOZ v1, each track is a fixed 6646-byte buffer with a trailing
/// bit count.  For WOZ v2, tracks have variable-length bitstreams addressed
/// by 512-byte block offsets.
///
/// Weak bits are detected as long runs of zero bits (>= 3 consecutive 0x00
/// bytes in the bitstream) which is the standard WOZ convention for marking
/// unreadable flux areas.
fn parse_woz(data: &[u8]) -> Option<WozImage> {
    if data.len() < 12 { return None; }

    let is_woz1 = data.starts_with(b"WOZ1\xff\x0a\x0d\x0a");
    let is_woz2 = data.starts_with(b"WOZ2\xff\x0a\x0d\x0a");
    if !is_woz1 && !is_woz2 { return None; }

    let version = if is_woz1 { 1u8 } else { 2u8 };

    // Validate CRC32: bytes 8..12 contain the stored CRC of bytes 12..end
    let stored_crc = u32::from_le_bytes(data[8..12].try_into().ok()?);
    if stored_crc != 0 {
        let computed = crc32(&data[12..]);
        if computed != stored_crc { return None; }
    }

    // Parse chunks
    let mut pos = 12usize;
    let mut tmap: Option<[u8; 160]> = None;
    let mut trks_data: &[u8] = &[];
    let mut write_protected = false;
    let mut disk_type: u8 = 1; // default 5.25"
    let mut synchronized = false;

    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
        pos += 8;
        if pos + size > data.len() { break; }
        let chunk = &data[pos..pos + size];

        match id {
            b"INFO" if size >= 60 => {
                // INFO chunk: version(1), disk_type(1), write_protected(1),
                //             synchronized(1), cleaned(1), creator(32), ...
                disk_type = chunk[1];
                write_protected = chunk[2] != 0;
                synchronized = chunk[3] != 0;
            }
            b"TMAP" if size >= 160 => {
                let mut t = [0xFFu8; 160];
                t.copy_from_slice(&chunk[..160]);
                tmap = Some(t);
            }
            b"TRKS" => {
                trks_data = chunk;
            }
            _ => {} // skip META and unknown chunks
        }

        pos += size;
    }

    let tmap = tmap?;

    // Determine the maximum track index referenced by TMAP
    let max_trks_idx = tmap.iter().copied().filter(|&v| v != 0xFF).max().unwrap_or(0) as usize;

    let mut tracks: Vec<Option<WozTrack>> = Vec::new();

    if is_woz1 {
        // WOZ1: TRKS chunk contains fixed 6656-byte entries (6646 bytes data + 2 bytes used,
        // plus 8 bytes padding = 6656 total per entry).
        const WOZ1_ENTRY_SIZE: usize = 6656;
        const WOZ1_DATA_SIZE: usize = 6646;

        for idx in 0..=max_trks_idx {
            let entry_off = idx * WOZ1_ENTRY_SIZE;
            if entry_off + WOZ1_ENTRY_SIZE > trks_data.len() {
                tracks.push(None);
                continue;
            }
            // Bytes used is at offset 6646 (2 bytes LE), bit count at 6648 (2 bytes LE)
            let bytes_used = u16::from_le_bytes(
                trks_data[entry_off + WOZ1_DATA_SIZE..entry_off + WOZ1_DATA_SIZE + 2]
                    .try_into().ok()?,
            ) as usize;
            let bit_count = u16::from_le_bytes(
                trks_data[entry_off + WOZ1_DATA_SIZE + 2..entry_off + WOZ1_DATA_SIZE + 4]
                    .try_into().ok()?,
            ) as u32;

            let byte_count = bytes_used.min(WOZ1_DATA_SIZE);
            let bits = trks_data[entry_off..entry_off + byte_count].to_vec();

            let (has_weak, weak_mask) = detect_weak_bits(&bits, bit_count);
            tracks.push(Some(WozTrack {
                bits,
                bit_count,
                has_weak_bits: has_weak,
                weak_mask,
            }));
        }
    } else {
        // WOZ2: TRKS chunk starts with 160 track descriptors (8 bytes each = 1280 bytes),
        // followed by track data in 512-byte blocks.
        const DESC_SIZE: usize = 8;

        for idx in 0..=max_trks_idx {
            let desc_off = idx * DESC_SIZE;
            if desc_off + DESC_SIZE > trks_data.len() {
                tracks.push(None);
                continue;
            }
            let starting_block = u16::from_le_bytes(
                trks_data[desc_off..desc_off + 2].try_into().ok()?,
            ) as usize;
            let block_count = u16::from_le_bytes(
                trks_data[desc_off + 2..desc_off + 4].try_into().ok()?,
            ) as usize;
            let bit_count = u32::from_le_bytes(
                trks_data[desc_off + 4..desc_off + 8].try_into().ok()?,
            );

            if starting_block == 0 && block_count == 0 {
                tracks.push(None);
                continue;
            }

            // Data addressed as 512-byte blocks relative to the file start
            let file_data_off = starting_block * 512;
            let byte_count = block_count * 512;
            if file_data_off + byte_count > data.len() {
                tracks.push(None);
                continue;
            }

            let bits = data[file_data_off..file_data_off + byte_count].to_vec();

            let (has_weak, weak_mask) = detect_weak_bits(&bits, bit_count);
            tracks.push(Some(WozTrack {
                bits,
                bit_count,
                has_weak_bits: has_weak,
                weak_mask,
            }));
        }
    }

    Some(WozImage {
        version,
        tracks,
        tmap,
        write_protected,
        disk_type,
        synchronized,
    })
}

/// Detect weak/flux bit areas in a bitstream.
///
/// The WOZ specification marks weak bits as long runs of zero bits.  Specifically,
/// any sequence of 3 or more consecutive 0x00 bytes (24+ zero bits) is considered
/// a weak bit area.  Returns (has_any, weak_mask) where weak_mask has 1-bits at
/// positions that should be randomized on read.
fn detect_weak_bits(bits: &[u8], bit_count: u32) -> (bool, Vec<u8>) {
    let byte_len = bits.len();
    let mut weak_mask = vec![0u8; byte_len];
    let mut has_any = false;

    // Scan for runs of 3+ consecutive 0x00 bytes
    let mut run_start: Option<usize> = None;

    for (i, &byte) in bits.iter().enumerate().take(byte_len) {
        if byte == 0x00 {
            if run_start.is_none() {
                run_start = Some(i);
            }
        } else {
            if let Some(start) = run_start {
                let run_len = i - start;
                if run_len >= 3 {
                    // Mark all bits in this run as weak
                    for (j, mask) in weak_mask.iter_mut().enumerate().take(i).skip(start) {
                        // Only mark bits within valid bit_count
                        if (j as u32) * 8 < bit_count {
                            *mask = 0xFF;
                            has_any = true;
                        }
                    }
                }
            }
            run_start = None;
        }
    }
    // Handle run extending to end of data
    if let Some(start) = run_start {
        let run_len = byte_len - start;
        if run_len >= 3 {
            for (j, mask) in weak_mask.iter_mut().enumerate().take(byte_len).skip(start) {
                if (j as u32) * 8 < bit_count {
                    *mask = 0xFF;
                    has_any = true;
                }
            }
        }
    }

    (has_any, weak_mask)
}

// ── Embedded 16-sector Disk II firmware (256 bytes) ──────────────────────────

static DISK2_FW: &[u8; 256] = &[
    0xA2, 0x20, 0xA0, 0x00, 0xA2, 0x03, 0x86, 0x3C,
    0x8A, 0x0A, 0x24, 0x3C, 0xF0, 0x10, 0x05, 0x3C,
    0x49, 0xFF, 0x29, 0x7E, 0xB0, 0x08, 0x4A, 0xD0,
    0xFB, 0x98, 0x9D, 0x56, 0x03, 0xC8, 0xE8, 0x10,
    0xE5, 0x20, 0x58, 0xFF, 0xBA, 0xBD, 0x00, 0x01,
    0x0A, 0x0A, 0x0A, 0x0A, 0x85, 0x2B, 0xAA, 0xBD,
    0x8E, 0xC0, 0xBD, 0x8C, 0xC0, 0xBD, 0x8A, 0xC0,
    0xBD, 0x89, 0xC0, 0xA0, 0x50, 0xBD, 0x80, 0xC0,
    0x98, 0x29, 0x03, 0x0A, 0x05, 0x2B, 0xAA, 0xBD,
    0x81, 0xC0, 0xA9, 0x56, 0x20, 0xA8, 0xFC, 0x88,
    0x10, 0xEB, 0x85, 0x26, 0x85, 0x3D, 0x85, 0x41,
    0xA9, 0x08, 0x85, 0x27, 0x18, 0x08, 0xBD, 0x8C,
    0xC0, 0x10, 0xFB, 0x49, 0xD5, 0xD0, 0xF7, 0xBD,
    0x8C, 0xC0, 0x10, 0xFB, 0xC9, 0xAA, 0xD0, 0xF3,
    0xEA, 0xBD, 0x8C, 0xC0, 0x10, 0xFB, 0xC9, 0x96,
    0xF0, 0x09, 0x28, 0x90, 0xDF, 0x49, 0xAD, 0xF0,
    0x25, 0xD0, 0xD9, 0xA0, 0x03, 0x85, 0x40, 0xBD,
    0x8C, 0xC0, 0x10, 0xFB, 0x2A, 0x85, 0x3C, 0xBD,
    0x8C, 0xC0, 0x10, 0xFB, 0x25, 0x3C, 0x88, 0xD0,
    0xEC, 0x28, 0xC5, 0x3D, 0xD0, 0xBE, 0xA5, 0x40,
    0xC5, 0x41, 0xD0, 0xB8, 0xB0, 0xB7, 0xA0, 0x56,
    0x84, 0x3C, 0xBC, 0x8C, 0xC0, 0x10, 0xFB, 0x59,
    0xD6, 0x02, 0xA4, 0x3C, 0x88, 0x99, 0x00, 0x03,
    0xD0, 0xEE, 0x84, 0x3C, 0xBC, 0x8C, 0xC0, 0x10,
    0xFB, 0x59, 0xD6, 0x02, 0xA4, 0x3C, 0x91, 0x26,
    0xC8, 0xD0, 0xEF, 0xBC, 0x8C, 0xC0, 0x10, 0xFB,
    0x59, 0xD6, 0x02, 0xD0, 0x87, 0xA0, 0x00, 0xA2,
    0x56, 0xCA, 0x30, 0xFB, 0xB1, 0x26, 0x5E, 0x00,
    0x03, 0x2A, 0x5E, 0x00, 0x03, 0x2A, 0x91, 0x26,
    0xC8, 0xD0, 0xEE, 0xE6, 0x27, 0xE6, 0x3D, 0xA5,
    0x3D, 0xCD, 0x00, 0x08, 0xA6, 0x2B, 0x90, 0xDB,
    0x4C, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00,
];

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper: build a minimal WOZ v1 image in memory ───────────────────

    /// Create a minimal WOZ v1 image with one track of known bit data.
    fn make_woz1_image(track_bits: &[u8], bit_count: u16) -> Vec<u8> {
        let mut img = Vec::new();

        // Magic
        img.extend_from_slice(b"WOZ1\xff\x0a\x0d\x0a");
        // CRC placeholder (will be filled in)
        img.extend_from_slice(&[0u8; 4]);

        // INFO chunk (60 bytes)
        img.extend_from_slice(b"INFO");
        img.extend_from_slice(&60u32.to_le_bytes());
        let mut info = [0u8; 60];
        info[0] = 1;  // version
        info[1] = 1;  // disk type = 5.25"
        info[2] = 0;  // not write protected
        info[3] = 0;  // not synchronized
        info[4] = 0;  // not cleaned
        // creator (32 bytes at offset 5)
        info[5..5 + 4].copy_from_slice(b"TEST");
        img.extend_from_slice(&info);

        // TMAP chunk (160 bytes)
        img.extend_from_slice(b"TMAP");
        img.extend_from_slice(&160u32.to_le_bytes());
        let mut tmap = [0xFFu8; 160];
        tmap[0] = 0; // quarter-track 0 -> TRKS index 0
        img.extend_from_slice(&tmap);

        // TRKS chunk: one entry = 6656 bytes
        // Layout: 6646 bytes data, 2 bytes bytes_used, 2 bytes bit_count, 8 bytes padding
        const ENTRY_SIZE: usize = 6656;
        let trks_size = ENTRY_SIZE;
        img.extend_from_slice(b"TRKS");
        img.extend_from_slice(&(trks_size as u32).to_le_bytes());

        let mut entry = vec![0u8; ENTRY_SIZE];
        let copy_len = track_bits.len().min(6646);
        entry[..copy_len].copy_from_slice(&track_bits[..copy_len]);
        // bytes_used at offset 6646
        entry[6646] = (copy_len & 0xFF) as u8;
        entry[6647] = ((copy_len >> 8) & 0xFF) as u8;
        // bit_count at offset 6648
        entry[6648] = (bit_count & 0xFF) as u8;
        entry[6649] = ((bit_count >> 8) & 0xFF) as u8;
        img.extend_from_slice(&entry);

        // Compute and fill CRC32
        let computed_crc = crc32(&img[12..]);
        img[8..12].copy_from_slice(&computed_crc.to_le_bytes());

        img
    }

    /// Create a minimal WOZ v2 image with one track of known bit data.
    fn make_woz2_image(track_bits: &[u8], bit_count: u32) -> Vec<u8> {
        let mut img = Vec::new();

        // Magic
        img.extend_from_slice(b"WOZ2\xff\x0a\x0d\x0a");
        // CRC placeholder
        img.extend_from_slice(&[0u8; 4]);

        // INFO chunk (60 bytes)
        img.extend_from_slice(b"INFO");
        img.extend_from_slice(&60u32.to_le_bytes());
        let mut info = [0u8; 60];
        info[0] = 2;  // version
        info[1] = 1;  // disk type = 5.25"
        info[2] = 1;  // write protected
        info[3] = 0;  // not synchronized
        info[4] = 0;  // not cleaned
        img.extend_from_slice(&info);

        // TMAP chunk (160 bytes)
        img.extend_from_slice(b"TMAP");
        img.extend_from_slice(&160u32.to_le_bytes());
        let mut tmap = [0xFFu8; 160];
        tmap[0] = 0;  // quarter-track 0 -> TRKS index 0
        tmap[4] = 0;  // quarter-track 4 also -> TRKS index 0 (shared)
        tmap[1] = 0;  // quarter-track 1 also -> TRKS index 0
        img.extend_from_slice(&tmap);

        // TRKS chunk: 160 track descriptors (8 bytes each = 1280),
        // followed by actual track data in 512-byte blocks.
        // Track data starts at block 3 (offset 3*512 = 1536 from file start).
        // The file so far: 12 + 68 + 168 + 8 = 256 bytes of header+chunks-before-TRKS
        // But TRKS data blocks are relative to the file, not the chunk.
        // We need to calculate: after writing TRKS chunk header + 1280 desc bytes,
        // where does the file offset land? That tells us the starting block.

        // Let's compute: current file size + 8 (TRKS header) + 1280 (descs)
        let trks_chunk_start = img.len() + 8; // position right after TRKS chunk header
        let descs_end = trks_chunk_start + 1280;
        // Round up to next 512-byte block
        let data_block_start = descs_end.div_ceil(512);
        let data_file_offset = data_block_start * 512;
        // Padding needed between descs and first data block
        let padding = data_file_offset - descs_end;

        // Calculate block count: ceil(track_bits.len() / 512)
        let block_count = track_bits.len().div_ceil(512) as u16;

        // Build descriptors: only index 0 is used
        let mut descs = vec![0u8; 1280];
        descs[0..2].copy_from_slice(&(data_block_start as u16).to_le_bytes());
        descs[2..4].copy_from_slice(&block_count.to_le_bytes());
        descs[4..8].copy_from_slice(&bit_count.to_le_bytes());

        let trks_total_size = 1280 + padding + (block_count as usize * 512);
        img.extend_from_slice(b"TRKS");
        img.extend_from_slice(&(trks_total_size as u32).to_le_bytes());
        img.extend_from_slice(&descs);
        img.extend(std::iter::repeat_n(0u8, padding));

        // Write track data padded to block_count * 512
        let mut track_data = track_bits.to_vec();
        track_data.resize(block_count as usize * 512, 0);
        img.extend_from_slice(&track_data);

        // Compute and fill CRC32
        let computed_crc = crc32(&img[12..]);
        img[8..12].copy_from_slice(&computed_crc.to_le_bytes());

        img
    }

    // ── CRC32 tests ──────────────────────────────────────────────────────

    #[test]
    fn test_crc32_known() {
        // CRC32 of "123456789" is 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn test_crc32_empty() {
        assert_eq!(crc32(b""), 0x0000_0000);
    }

    // ── WOZ v1 parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_woz1_basic() {
        // Create a simple bitstream: alternating 1-0 pattern = 0xAA
        let track_data = vec![0xAA; 100];
        let bit_count = 800u16; // 100 bytes * 8 bits
        let img = make_woz1_image(&track_data, bit_count);

        let woz = parse_woz(&img).expect("should parse WOZ v1");
        assert_eq!(woz.version, 1);
        assert_eq!(woz.disk_type, 1);
        assert!(!woz.write_protected);

        // Quarter-track 0 should map to track index 0
        assert_eq!(woz.tmap[0], 0);
        assert_eq!(woz.tmap[1], 0xFF); // unmapped

        let track = woz.track_for_quarter(0).expect("track 0 should exist");
        assert_eq!(track.bit_count, 800);
        assert_eq!(track.bits[0], 0xAA);
    }

    #[test]
    fn test_parse_woz1_bad_magic() {
        let mut img = make_woz1_image(&[0xFF; 10], 80);
        img[0..4].copy_from_slice(b"NOPE");
        assert!(parse_woz(&img).is_none());
    }

    #[test]
    fn test_parse_woz1_bad_crc() {
        let mut img = make_woz1_image(&[0xFF; 10], 80);
        // Corrupt the CRC
        img[8] = img[8].wrapping_add(1);
        assert!(parse_woz(&img).is_none());
    }

    // ── WOZ v2 parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_woz2_basic() {
        let track_data = vec![0xD5; 64];
        let bit_count = 512u32;
        let img = make_woz2_image(&track_data, bit_count);

        let woz = parse_woz(&img).expect("should parse WOZ v2");
        assert_eq!(woz.version, 2);
        assert!(woz.write_protected);

        let track = woz.track_for_quarter(0).expect("track 0 should exist");
        assert_eq!(track.bit_count, 512);
    }

    #[test]
    fn test_parse_woz2_quarter_track_mapping() {
        let track_data = vec![0xFF; 64];
        let img = make_woz2_image(&track_data, 512);

        let woz = parse_woz(&img).expect("should parse");
        // Quarter-track 0, 1, and 4 all map to track index 0
        assert!(woz.track_for_quarter(0).is_some());
        assert!(woz.track_for_quarter(1).is_some());
        assert!(woz.track_for_quarter(4).is_some());
        // Quarter-track 2 is unmapped
        assert!(woz.track_for_quarter(2).is_none());
        // Out of range
        assert!(woz.track_for_quarter(160).is_none());
    }

    // ── Bit-level read tests ─────────────────────────────────────────────

    #[test]
    fn test_woz_read_bit() {
        // Bitstream: 0b10110100 = 0xB4
        let track = WozTrack {
            bits: vec![0xB4],
            bit_count: 8,
            has_weak_bits: false,
            weak_mask: vec![0],
        };
        let mut rng = SimpleRng::new(42);

        assert_eq!(track.read_bit(0, &mut rng), 1); // bit 7
        assert_eq!(track.read_bit(1, &mut rng), 0); // bit 6
        assert_eq!(track.read_bit(2, &mut rng), 1); // bit 5
        assert_eq!(track.read_bit(3, &mut rng), 1); // bit 4
        assert_eq!(track.read_bit(4, &mut rng), 0); // bit 3
        assert_eq!(track.read_bit(5, &mut rng), 1); // bit 2
        assert_eq!(track.read_bit(6, &mut rng), 0); // bit 1
        assert_eq!(track.read_bit(7, &mut rng), 0); // bit 0
    }

    #[test]
    fn test_woz_read_nibble_shift_register() {
        // Create a bitstream that encodes nibble 0xD5:
        // 0xD5 = 0b11010101
        // The shift register accumulates bits until bit 7 is set.
        // So bits: 1,1,0,1,0,1,0,1 -> first 1 starts, then 7 more -> 0xD5
        let track = WozTrack {
            bits: vec![0xD5, 0xAA], // 0xD5 followed by 0xAA
            bit_count: 16,
            has_weak_bits: false,
            weak_mask: vec![0, 0],
        };

        let woz = WozImage {
            version: 1,
            tracks: vec![Some(track)],
            tmap: {
                let mut t = [0xFFu8; 160];
                t[0] = 0;
                t
            },
            write_protected: false,
            disk_type: 1,
            synchronized: false,
        };

        let mut drive = Drive::new();
        drive.woz = Some(woz);
        drive.loaded = true;
        drive.last_cycle = 0;

        // Reading a nibble with enough cycles elapsed (8 bits * 4 cycles = 32 cycles)
        let nibble = drive.woz_read_nibble(32);
        assert_eq!(nibble, Some(0xD5));
    }

    // ── Bit-level write tests ────────────────────────────────────────────

    #[test]
    fn test_woz_write_bit() {
        let mut track = WozTrack {
            bits: vec![0x00],
            bit_count: 8,
            has_weak_bits: false,
            weak_mask: vec![0],
        };

        // Write 1 at position 0 (MSB)
        track.write_bit(0, 1);
        assert_eq!(track.bits[0], 0x80);

        // Write 1 at position 7 (LSB)
        track.write_bit(7, 1);
        assert_eq!(track.bits[0], 0x81);

        // Write 0 at position 0 (clear MSB)
        track.write_bit(0, 0);
        assert_eq!(track.bits[0], 0x01);
    }

    #[test]
    fn test_woz_write_nibble() {
        let track = WozTrack {
            bits: vec![0x00; 4],
            bit_count: 32,
            has_weak_bits: false,
            weak_mask: vec![0; 4],
        };

        let woz = WozImage {
            version: 1,
            tracks: vec![Some(track)],
            tmap: {
                let mut t = [0xFFu8; 160];
                t[0] = 0;
                t
            },
            write_protected: false,
            disk_type: 1,
            synchronized: false,
        };

        let mut drive = Drive::new();
        drive.woz = Some(woz);
        drive.loaded = true;
        drive.last_cycle = 0;

        // Write nibble 0xD5 starting at bit position 0
        drive.woz_write_nibble(0xD5, 0);
        assert!(drive.dirty);

        // Check the bits: 0xD5 = 11010101 should be written at positions 0..8
        let track = drive.woz.as_ref().unwrap().track_for_quarter(0).unwrap();
        assert_eq!(track.bits[0], 0xD5);
    }

    // ── Weak bit tests ───────────────────────────────────────────────────

    #[test]
    fn test_detect_weak_bits() {
        // 3 consecutive zero bytes = weak area
        let bits = vec![0xFF, 0x00, 0x00, 0x00, 0xFF];
        let (has_weak, mask) = detect_weak_bits(&bits, 40);
        assert!(has_weak);
        assert_eq!(mask[0], 0x00); // not weak
        assert_eq!(mask[1], 0xFF); // weak
        assert_eq!(mask[2], 0xFF); // weak
        assert_eq!(mask[3], 0xFF); // weak
        assert_eq!(mask[4], 0x00); // not weak
    }

    #[test]
    fn test_detect_weak_bits_short_run() {
        // Only 2 consecutive zero bytes = NOT weak
        let bits = vec![0xFF, 0x00, 0x00, 0xFF];
        let (has_weak, _mask) = detect_weak_bits(&bits, 32);
        assert!(!has_weak);
    }

    #[test]
    fn test_weak_bit_randomization() {
        let track = WozTrack {
            bits: vec![0x00, 0x00, 0x00], // all zeros = weak area
            bit_count: 24,
            has_weak_bits: true,
            weak_mask: vec![0xFF, 0xFF, 0xFF],
        };

        let mut rng = SimpleRng::new(12345);

        // Read several weak bits — they should not all be the same value
        let mut seen_zero = false;
        let mut seen_one = false;
        for _ in 0..100 {
            let bit = track.read_bit(0, &mut rng);
            if bit == 0 { seen_zero = true; }
            if bit == 1 { seen_one = true; }
            if seen_zero && seen_one { break; }
        }
        assert!(seen_zero && seen_one, "weak bits should produce both 0 and 1");
    }

    #[test]
    fn test_write_clears_weak_bits() {
        let mut track = WozTrack {
            bits: vec![0x00, 0x00, 0x00],
            bit_count: 24,
            has_weak_bits: true,
            weak_mask: vec![0xFF, 0xFF, 0xFF],
        };

        // Write a 1 at position 0
        track.write_bit(0, 1);
        // The weak mask at byte 0, bit 7 should now be cleared
        assert_eq!(track.weak_mask[0] & 0x80, 0);
    }

    // ── SimpleRng tests ──────────────────────────────────────────────────

    #[test]
    fn test_simple_rng_produces_both_values() {
        let mut rng = SimpleRng::new(42);
        let mut seen = [false; 2];
        for _ in 0..100 {
            seen[rng.next_bit() as usize] = true;
            if seen[0] && seen[1] { break; }
        }
        assert!(seen[0] && seen[1]);
    }

    #[test]
    fn test_simple_rng_deterministic() {
        let mut rng1 = SimpleRng::new(999);
        let mut rng2 = SimpleRng::new(999);
        for _ in 0..50 {
            assert_eq!(rng1.next(), rng2.next());
        }
    }

    // ── Disk2Card integration tests ──────────────────────────────────────

    #[test]
    fn test_load_woz1_into_card() {
        let track_data = vec![0xD5; 100];
        let img = make_woz1_image(&track_data, 800);

        let mut card = Disk2Card::new(6);
        assert!(card.load_drive(0, &img, "woz"));
        assert!(card.drives[0].is_woz());
        assert!(card.drives[0].loaded);
    }

    #[test]
    fn test_load_woz2_into_card() {
        let track_data = vec![0xAA; 128];
        let img = make_woz2_image(&track_data, 1024);

        let mut card = Disk2Card::new(6);
        assert!(card.load_drive(0, &img, "woz"));
        assert!(card.drives[0].is_woz());
        assert!(card.drives[0].write_protected); // WOZ v2 test image is write-protected
    }

    #[test]
    fn test_autodetect_woz_by_magic() {
        let track_data = vec![0xFF; 100];
        let img = make_woz1_image(&track_data, 800);

        let mut card = Disk2Card::new(6);
        // Even with "dsk" extension, WOZ magic should be auto-detected
        assert!(card.load_drive(0, &img, "dsk"));
        assert!(card.drives[0].is_woz());
    }

    #[test]
    fn test_dsk_still_works() {
        // Ensure a normal DSK image still loads in nibble mode
        let dsk = vec![0x00u8; DSK_SIZE];
        let mut card = Disk2Card::new(6);
        assert!(card.load_drive(0, &dsk, "dsk"));
        assert!(!card.drives[0].is_woz());
        assert!(card.drives[0].loaded);
    }

    #[test]
    fn test_woz_eject() {
        let track_data = vec![0xFF; 100];
        let img = make_woz1_image(&track_data, 800);

        let mut card = Disk2Card::new(6);
        card.load_drive(0, &img, "woz");
        card.eject_drive(0);
        assert!(!card.drives[0].loaded);
    }

    // ── Bit timing tests ─────────────────────────────────────────────────

    #[test]
    fn test_advance_bits_timing() {
        let mut drive = Drive::new();
        drive.last_cycle = 100;

        // 16 cycles elapsed = 4 bits (at 4 cycles per bit)
        let bits = drive.advance_bits(116);
        assert_eq!(bits, 4);
        assert_eq!(drive.last_cycle, 116);
    }

    #[test]
    fn test_advance_bits_no_time() {
        let mut drive = Drive::new();
        drive.last_cycle = 100;
        assert_eq!(drive.advance_bits(100), 0);
        assert_eq!(drive.advance_bits(50), 0); // backwards = 0
    }

    // ── Track wrap-around test ───────────────────────────────────────────

    #[test]
    fn test_bit_position_wraps() {
        let track = WozTrack {
            bits: vec![0xFF, 0x00], // 16 bits
            bit_count: 16,
            has_weak_bits: false,
            weak_mask: vec![0, 0],
        };

        let woz = WozImage {
            version: 1,
            tracks: vec![Some(track)],
            tmap: {
                let mut t = [0xFFu8; 160];
                t[0] = 0;
                t
            },
            write_protected: false,
            disk_type: 1,
            synchronized: false,
        };

        let mut drive = Drive::new();
        drive.woz = Some(woz);
        drive.loaded = true;
        drive.bit_pos = 15; // at last bit

        // Read one bit, should wrap to position 0
        let _bit = drive.woz_read_bit();
        assert_eq!(drive.bit_pos, 0); // wrapped around
    }
}
