//! Disk II interface card — 16-sector, .dsk/.do/.po/.nib/.woz image support.
//!
//! Implements the Disk II controller as described in "Beneath Apple DOS"
//! and translated from `source/Disk.cpp` / `source/DiskImageHelper.cpp`.

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

// ── Per-drive state ───────────────────────────────────────────────────────────

struct Drive {
    /// Pre-nibblized track data: 35 entries, each a GCR byte stream.
    tracks:          Vec<Vec<u8>>,
    /// Head position in quarter-tracks (0–79; integer track = phase / 2).
    phase:           i32,
    /// Cached integer track index (= phase / 2, clamped to 0..NUM_TRACKS-1).
    /// Updated whenever `phase` changes to avoid recomputing it in the hot
    /// nibble read/write path.
    current_track_idx: usize,
    /// Current byte offset within the current track buffer.
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
        }
    }

    /// Recompute `current_track_idx` from `phase`.  Call after any write to `phase`.
    #[inline]
    fn update_track_idx(&mut self) {
        self.current_track_idx = (self.phase / 2).clamp(0, (NUM_TRACKS as i32) - 1) as usize;
    }

    /// Return the next nibble and advance the byte pointer.
    fn read_nibble(&mut self) -> u8 {
        if !self.loaded { return 0xFF; }
        let buf = &self.tracks[self.current_track_idx];
        if buf.is_empty() { return 0xFF; }
        let n = buf[self.byte_pos];
        self.byte_pos = (self.byte_pos + 1) % buf.len();
        n
    }

    fn write_nibble(&mut self, byte: u8) {
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
                // WOZ write-back not supported — would need to repack bitstream
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
            latch:        0,
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
    /// `"nib"` → raw nibbles, `"woz"` → WOZ bitstream (write-protected).
    pub fn load_drive(&mut self, drive: usize, data: &[u8], ext: &str) -> bool {
        if drive >= 2 { return false; }
        let (format, tracks, write_protected) = match ext {
            "woz" => {
                let t = load_woz(data);
                (DiskFormat::Woz, t, true)
            }
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
            self.drives[drive].tracks            = t;
            self.drives[drive].loaded            = true;
            self.drives[drive].byte_pos          = 0;
            self.drives[drive].phase             = 0;
            self.drives[drive].current_track_idx = 0;
            self.drives[drive].dirty             = false;
            self.drives[drive].format            = format;
            self.drives[drive].raw               = data.to_vec();
            self.drives[drive].write_protected   = write_protected;
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
        let fwd   = if self.phases & (1 << ((cur + 1) & 3)) != 0 {  1i32 } else { 0 };
        let bwd   = if self.phases & (1 << ((cur + 3) & 3)) != 0 { -1i32 } else { 0 };
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

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
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
                    self.drives[self.active_drive].write_nibble(self.latch);
                } else if !self.write_mode && self.motor_on {
                    self.latch = self.drives[self.active_drive].read_nibble();
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

    fn slot_io_write(&mut self, reg: u8, value: u8, _cycles: u64) {
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
                    self.drives[self.active_drive].write_nibble(value);
                }
            }
            0x0D => { self.write_mode = false; }
            0x0F => { self.write_mode = true; }
            _    => {}
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.motor_on     = false;
        self.write_mode   = false;
        self.latch        = 0;
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

// ── WOZ format support ────────────────────────────────────────────────────────

/// Parse a WOZ disk image and return nibble tracks.
/// WOZ is write-protected in emulation.
fn load_woz(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    if data.len() < 12 { return None; }

    // Check magic: "WOZ1\xFF\x0A\x0D\x0A" or "WOZ2\xFF\x0A\x0D\x0A"
    let is_woz1 = data.starts_with(b"WOZ1\xff\x0a\x0d\x0a");
    let is_woz2 = data.starts_with(b"WOZ2\xff\x0a\x0d\x0a");
    if !is_woz1 && !is_woz2 { return None; }

    // Skip 8-byte magic + 4-byte CRC = offset 12 for first chunk
    let mut pos = 12usize;

    let mut tmap: Option<[u8; 160]> = None;
    let mut trks_data: &[u8] = &[];

    // Parse chunks
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
        pos += 8;
        if pos + size > data.len() { break; }
        let chunk = &data[pos..pos + size];

        if id == b"TMAP" && size >= 160 {
            let mut t = [0xFFu8; 160];
            t.copy_from_slice(&chunk[..160]);
            tmap = Some(t);
        } else if id == b"TRKS" {
            trks_data = chunk;
        }

        pos += size;
    }

    let tmap = tmap?;

    // Build nibble tracks for the 35 whole tracks (0, 2, 4, ... 68 in quarter-track space)
    let mut tracks = vec![Vec::new(); NUM_TRACKS];

    if is_woz1 {
        // WOZ1: each TRKS entry is 6646 bytes of bitstream
        const WOZ1_TRACK_BYTES: usize = 6646;
        const WOZ1_BIT_COUNT:   usize = 6646 * 8; // max bits
        #[allow(clippy::needless_range_loop)]
        for track_idx in 0..NUM_TRACKS {
            let qt = track_idx * 2; // quarter-track index for whole track
            let trks_idx = tmap[qt];
            if trks_idx == 0xFF { continue; }
            let t_off = trks_idx as usize * WOZ1_TRACK_BYTES;
            if t_off + WOZ1_TRACK_BYTES > trks_data.len() { continue; }
            let bits = &trks_data[t_off..t_off + WOZ1_TRACK_BYTES];
            tracks[track_idx] = bitstream_to_nibbles(bits, WOZ1_BIT_COUNT);
        }
    } else {
        // WOZ2: 8-byte track descriptors at start of TRKS, data blocks at offset 1536
        // Track descriptor: u16 starting_block, u16 block_count, u32 bit_count
        const DESC_SIZE: usize = 8;
        #[allow(clippy::needless_range_loop)]
        for track_idx in 0..NUM_TRACKS {
            let qt = track_idx * 2;
            let trks_idx = tmap[qt];
            if trks_idx == 0xFF { continue; }
            let desc_off = trks_idx as usize * DESC_SIZE;
            if desc_off + DESC_SIZE > trks_data.len() { continue; }
            let starting_block = u16::from_le_bytes(trks_data[desc_off..desc_off + 2].try_into().ok()?) as usize;
            let block_count    = u16::from_le_bytes(trks_data[desc_off + 2..desc_off + 4].try_into().ok()?) as usize;
            let bit_count      = u32::from_le_bytes(trks_data[desc_off + 4..desc_off + 8].try_into().ok()?) as usize;

            // Data starts addressed in 512-byte blocks relative to the beginning of the file.
            let file_data_off = starting_block * 512;
            let byte_count = block_count * 512;
            if file_data_off + byte_count > data.len() { continue; }
            let bits = &data[file_data_off..file_data_off + byte_count];
            tracks[track_idx] = bitstream_to_nibbles(bits, bit_count);
        }
    }

    Some(tracks)
}

/// Convert a raw bitstream (MSB-first within each byte) to nibbles.
/// Simulates the Apple II disk controller's shift register: accumulate bits
/// until the high bit is set, then emit the byte.
fn bitstream_to_nibbles(bits: &[u8], bit_count: usize) -> Vec<u8> {
    let mut nibs = Vec::with_capacity(bits.len());
    let mut shift_reg: u8 = 0;

    // Double the bitstream to handle wrap-around (tracks are circular)
    let total = bit_count.min(bits.len() * 8);

    for bit_idx in 0..total * 2 {
        let actual_bit = bit_idx % total;
        let byte_idx = actual_bit / 8;
        let bit_pos  = 7 - (actual_bit % 8); // MSB first
        let bit = (bits[byte_idx] >> bit_pos) & 1;

        shift_reg = (shift_reg << 1) | bit;
        if shift_reg & 0x80 != 0 {
            nibs.push(shift_reg);
            shift_reg = 0;
            if bit_idx >= total {
                // We've wrapped once; enough data for one revolution
                break;
            }
        }
    }

    nibs
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
