//! Hard Disk / SmartPort card emulation.
//!
//! Emulates the AppleWin "hddrvr" v1 block-device firmware interface, giving
//! ProDOS access to `.hdv` / `.po` / `.2mg` disk images as 512-byte block
//! devices.  Up to two drives (drive 0 = primary, drive 1 = secondary).
//!
//! Memory map (IO addr + slot*$10):
//!   C080  (r)   EXECUTE — triggers current command; returns status
//!   C081  (r)   STATUS  — b7=busy, b0=error
//!   C082  (r/w) COMMAND — 0=STATUS, 1=READ, 2=WRITE, 3=FORMAT
//!   C083  (r/w) UNIT    — b7=drive (0/1), b6..4=slot
//!   C084  (r/w) BUF_LO  — destination / source buffer address (low byte)
//!   C085  (r/w) BUF_HI  — buffer address (high byte)
//!   C086  (r/w) BLK_LO  — block number (low byte)
//!   C087  (r/w) BLK_HI  — block number (high byte)
//!   C089  (r)   SIZE_LO — disk size in blocks (low byte)
//!   C08A  (r)   SIZE_HI — disk size in blocks (high byte)
//!
//! Reference: source/Harddisk.cpp, resource/Hddrvr.bin

use crate::card::{Card, CardType, DmaWrite, DriveActivity};
use crate::error::Result;
use std::io::{Read, Write};

// ── Error codes (ProDOS block device error codes) ─────────────────────────────

const ERR_OK: u8 = 0x00;
const ERR_IO: u8 = 0x27;
const ERR_NO_DEVICE: u8 = 0x28;

// ── Firmware ROM (embedded from resource/Hddrvr.bin) ─────────────────────────
//
// The firmware contains the correct ProDOS ID bytes at required offsets,
// allowing ProDOS to auto-discover the card in any slot.

static HD_FIRMWARE: &[u8; 256] = include_bytes!("../../../../roms/Hddrvr.bin");

// ── Commands ──────────────────────────────────────────────────────────────────

const CMD_STATUS: u8 = 0x00;
const CMD_READ: u8 = 0x01;
const CMD_WRITE: u8 = 0x02;

/// 512-byte ProDOS block size.
const BLOCK_SIZE: usize = 512;

// ── Drive state ───────────────────────────────────────────────────────────────

struct Drive {
    /// Raw image bytes (block-ordered, 512 bytes per block).
    image: Vec<u8>,
}

impl Drive {
    fn block_count(&self) -> u32 {
        (self.image.len() / BLOCK_SIZE) as u32
    }

    fn read_block(&self, block: u32) -> Option<&[u8]> {
        let off = block as usize * BLOCK_SIZE;
        if off + BLOCK_SIZE <= self.image.len() {
            Some(&self.image[off..off + BLOCK_SIZE])
        } else {
            None
        }
    }

    fn write_block(&mut self, block: u32, data: &[u8]) -> bool {
        let off = block as usize * BLOCK_SIZE;
        if data.len() < BLOCK_SIZE {
            return false;
        }
        // Grow the image if needed
        let needed = off + BLOCK_SIZE;
        if needed > self.image.len() {
            self.image.resize(needed, 0);
        }
        self.image[off..off + BLOCK_SIZE].copy_from_slice(&data[..BLOCK_SIZE]);
        true
    }
}

// ── HdCard ────────────────────────────────────────────────────────────────────

/// Maximum number of SmartPort drives supported.
pub const MAX_HD_DRIVES: usize = 8;

pub struct HdCard {
    slot: usize,
    drives: [Option<Drive>; MAX_HD_DRIVES],

    // I/O registers
    command: u8,
    unit: u8,
    buf_lo: u8,
    buf_hi: u8,
    blk_lo: u8,
    blk_hi: u8,

    // Execution result
    status: u8, // b0 = error flag

    // Pending DMA write (card → Apple II RAM) after a successful READ
    dma_write: Option<DmaWrite>,
    // Pending DMA read request (Apple II RAM → card) before a WRITE
    dma_read_req: Option<(u16, u16)>, // (src_addr, len)
    dma_write_buf: Option<Vec<u8>>,   // received RAM data for WRITE
    dma_write_blk: Option<u32>,       // block number for the pending WRITE

    /// Countdown of update ticks remaining for the activity LED.
    /// Set on each command execution, decremented by `update()`.
    activity_ticks: u32,
    /// True when the last command was a write (LED colour hint).
    last_was_write: bool,
}

impl HdCard {
    pub fn new(slot: usize) -> Self {
        Self {
            slot,
            drives: [None, None, None, None, None, None, None, None],
            command: 0,
            unit: 0,
            buf_lo: 0,
            buf_hi: 0,
            blk_lo: 0,
            blk_hi: 0,
            status: 0,
            dma_write: None,
            dma_read_req: None,
            dma_write_buf: None,
            dma_write_blk: None,
            activity_ticks: 0,
            last_was_write: false,
        }
    }

    pub fn load_image(&mut self, drive: usize, data: Vec<u8>) {
        if drive < MAX_HD_DRIVES {
            // Decompress if gzip/zip, then unwrap 2IMG header if present.
            let (data, _ext) = crate::disk_util::decompress(&data, "hdv");
            let data = match crate::disk_util::unwrap_2img(&data) {
                Some((inner, _fmt)) => inner,
                None => data,
            };
            self.drives[drive] = Some(Drive { image: data });
        }
    }

    pub fn eject(&mut self, drive: usize) {
        if drive < MAX_HD_DRIVES {
            self.drives[drive] = None;
        }
    }

    pub fn take_image(&self, drive: usize) -> Option<&[u8]> {
        self.drives.get(drive)?.as_ref().map(|d| d.image.as_slice())
    }

    fn drive_idx(&self) -> usize {
        // SmartPort unit byte: bits 7:4 encode the drive number.
        // For backwards compatibility, bit 7 alone selects drive 0 or 1.
        // Extended: bits 6:4 give drives 0–7 when bit 7 is combined.
        let idx = ((self.unit >> 4) & 0x0F) as usize;
        idx.min(MAX_HD_DRIVES - 1)
    }

    fn buf_addr(&self) -> u16 {
        (self.buf_hi as u16) << 8 | self.buf_lo as u16
    }

    fn blk_num(&self) -> u32 {
        (self.blk_hi as u32) << 8 | self.blk_lo as u32
    }

    fn block_count(&self, drive: usize) -> u32 {
        self.drives[drive]
            .as_ref()
            .map(|d| d.block_count())
            .unwrap_or(0)
    }

    /// Execute the current command.  Returns the status byte.
    fn execute(&mut self) -> u8 {
        // Light the activity LED for ~100ms (~6 update ticks at 60 Hz)
        self.activity_ticks = 6;
        self.last_was_write = self.command == CMD_WRITE;
        let drv = self.drive_idx();
        match self.command {
            CMD_STATUS => {
                if self.drives[drv].is_none() {
                    self.status = ERR_NO_DEVICE;
                } else {
                    self.status = ERR_OK;
                }
                self.status
            }
            CMD_READ => {
                let blk = self.blk_num();
                if let Some(drive) = &self.drives[drv] {
                    if let Some(data) = drive.read_block(blk) {
                        // Schedule DMA write: copy block data to Apple II RAM
                        self.dma_write = Some(DmaWrite {
                            dest: self.buf_addr(),
                            data: data.to_vec(),
                        });
                        self.status = ERR_OK;
                    } else {
                        self.status = ERR_IO;
                    }
                } else {
                    self.status = ERR_NO_DEVICE;
                }
                self.status
            }
            CMD_WRITE => {
                // Request DMA read: get 512 bytes from Apple II RAM at buf_addr
                self.dma_read_req = Some((self.buf_addr(), BLOCK_SIZE as u16));
                self.dma_write_blk = Some(self.blk_num());
                self.status = ERR_OK;
                self.status
            }
            _ => {
                // FORMAT or unknown: return OK (no-op)
                self.status = ERR_OK;
                self.status
            }
        }
    }
}

impl Card for HdCard {
    fn card_type(&self) -> CardType {
        CardType::GenericHdd
    }
    fn slot(&self) -> usize {
        self.slot
    }

    fn io_read(&mut self, offset: u8, _cycles: u64) -> u8 {
        // $Cn00–$CnFF: firmware ROM
        *HD_FIRMWARE.get(offset as usize).unwrap_or(&0xFF)
    }

    fn io_write(&mut self, _offset: u8, _value: u8, _cycles: u64) {}

    fn cx_rom(&self) -> Option<&[u8; 256]> {
        Some(HD_FIRMWARE)
    }

    fn slot_io_read(&mut self, reg: u8, _cycles: u64) -> u8 {
        match reg {
            0x0 => {
                // EXECUTE: trigger command and return status
                self.execute()
            }
            0x1 => self.status,
            0x2 => self.command,
            0x3 => self.unit,
            0x4 => self.buf_lo,
            0x5 => self.buf_hi,
            0x6 => self.blk_lo,
            0x7 => self.blk_hi,
            0x9 => {
                // SIZE_LO
                let bc = self.block_count(self.drive_idx());
                (bc & 0xFF) as u8
            }
            0xA => {
                // SIZE_HI
                let bc = self.block_count(self.drive_idx());
                ((bc >> 8) & 0xFF) as u8
            }
            _ => 0xFF,
        }
    }

    fn slot_io_write(&mut self, reg: u8, val: u8, _cycles: u64) {
        match reg {
            0x2 => self.command = val,
            0x3 => self.unit = val,
            0x4 => self.buf_lo = val,
            0x5 => self.buf_hi = val,
            0x6 => self.blk_lo = val,
            0x7 => self.blk_hi = val,
            _ => {}
        }
    }

    fn take_dma_write(&mut self) -> Option<DmaWrite> {
        self.dma_write.take()
    }

    fn take_dma_read_request(&mut self) -> Option<(u16, u16)> {
        self.dma_read_req.take()
    }

    fn dma_read_complete(&mut self, data: &[u8]) {
        // We have the Apple II RAM data — perform the disk write now
        if let Some(blk) = self.dma_write_blk.take() {
            let drv = self.drive_idx();
            if let Some(drive) = &mut self.drives[drv] {
                if !drive.write_block(blk, data) {
                    self.status = ERR_IO;
                }
            } else {
                self.status = ERR_NO_DEVICE;
            }
        }
    }

    fn reset(&mut self, _power_cycle: bool) {
        self.command = 0;
        self.status = 0;
        self.dma_write = None;
        self.dma_read_req = None;
        self.dma_write_buf = None;
        self.dma_write_blk = None;
    }

    fn update(&mut self, _cycles: u64) {
        self.activity_ticks = self.activity_ticks.saturating_sub(1);
    }

    fn save_state(&self, _out: &mut dyn Write) -> Result<()> {
        Ok(())
    }
    fn load_state(&mut self, _src: &mut dyn Read, _version: u32) -> Result<()> {
        Ok(())
    }

    fn disk_drive_activity(&self, _drive: usize) -> DriveActivity {
        DriveActivity {
            motor_on: self.activity_ticks > 0,
            writing: self.activity_ticks > 0 && self.last_was_write,
            track: 0,
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
