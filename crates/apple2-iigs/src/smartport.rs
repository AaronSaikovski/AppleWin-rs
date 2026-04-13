//! Apple IIgs SmartPort disk interface.
//!
//! SmartPort provides block-level access to 3.5" (800KB) and hard disk images.
//! The IIgs ROM firmware handles the SmartPort protocol; the emulator provides
//! the underlying block storage.
//!
//! Supported image formats:
//! - `.2mg` / `.2img`: 2IMG container (800KB or larger)
//! - `.po`: ProDOS-order raw block image
//! - `.hdv`: Hard disk volume image

use serde::{Deserialize, Serialize};

/// Block size in bytes (always 512 for ProDOS).
pub const BLOCK_SIZE: usize = 512;

/// SmartPort device type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceType {
    /// 3.5" floppy (800KB = 1600 blocks).
    Floppy35,
    /// Hard disk / large volume.
    HardDisk,
}

/// A SmartPort disk image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartPortDisk {
    /// Device type.
    pub device_type: DeviceType,
    /// Raw block data.
    pub data: Vec<u8>,
    /// Number of blocks.
    pub num_blocks: u32,
    /// Write-protected flag.
    pub write_protected: bool,
    /// File path (for display/save).
    pub path: Option<String>,
}

impl SmartPortDisk {
    /// Create a new SmartPort disk from raw image data.
    pub fn from_raw(data: Vec<u8>, path: Option<String>) -> Self {
        let num_blocks = (data.len() / BLOCK_SIZE) as u32;
        let device_type = if num_blocks <= 1600 {
            DeviceType::Floppy35
        } else {
            DeviceType::HardDisk
        };

        Self {
            device_type,
            data,
            num_blocks,
            write_protected: false,
            path,
        }
    }

    /// Create from a .2mg file, stripping the header.
    pub fn from_2mg(raw: &[u8], path: Option<String>) -> Option<Self> {
        // 2IMG header is 64 bytes minimum
        if raw.len() < 64 {
            return None;
        }

        // Check magic: "2IMG"
        if &raw[0..4] != b"2IMG" {
            return None;
        }

        // Data offset (4 bytes at offset 8, little-endian)
        let data_offset = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as usize;
        // Data length (4 bytes at offset 12)
        let data_len = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]) as usize;

        if data_offset + data_len > raw.len() {
            return None;
        }

        let data = raw[data_offset..data_offset + data_len].to_vec();
        Some(Self::from_raw(data, path))
    }

    /// Read a block (512 bytes). Returns None if block number is out of range.
    pub fn read_block(&self, block: u32) -> Option<&[u8]> {
        if block >= self.num_blocks {
            return None;
        }
        let offset = block as usize * BLOCK_SIZE;
        Some(&self.data[offset..offset + BLOCK_SIZE])
    }

    /// Write a block (512 bytes). Returns false if write-protected or out of range.
    pub fn write_block(&mut self, block: u32, data: &[u8]) -> bool {
        if self.write_protected || block >= self.num_blocks || data.len() != BLOCK_SIZE {
            return false;
        }
        let offset = block as usize * BLOCK_SIZE;
        self.data[offset..offset + BLOCK_SIZE].copy_from_slice(data);
        true
    }
}

/// SmartPort controller managing up to 4 devices.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SmartPort {
    /// Attached disk images (up to 4 devices).
    pub disks: [Option<SmartPortDisk>; 4],
}

impl SmartPort {
    /// Insert a disk into a drive slot (0-3).
    pub fn insert(&mut self, drive: usize, disk: SmartPortDisk) {
        if drive < 4 {
            self.disks[drive] = Some(disk);
        }
    }

    /// Eject a disk from a drive slot.
    pub fn eject(&mut self, drive: usize) {
        if drive < 4 {
            self.disks[drive] = None;
        }
    }

    /// Read a block from a device.
    pub fn read_block(&self, device: usize, block: u32) -> Option<Vec<u8>> {
        self.disks
            .get(device)?
            .as_ref()?
            .read_block(block)
            .map(|b| b.to_vec())
    }

    /// Write a block to a device.
    pub fn write_block(&mut self, device: usize, block: u32, data: &[u8]) -> bool {
        if let Some(Some(disk)) = self.disks.get_mut(device) {
            disk.write_block(block, data)
        } else {
            false
        }
    }

    /// Get the number of blocks on a device, or 0 if no disk.
    pub fn device_blocks(&self, device: usize) -> u32 {
        self.disks
            .get(device)
            .and_then(|d| d.as_ref())
            .map(|d| d.num_blocks)
            .unwrap_or(0)
    }

    /// Check if a device has a disk inserted.
    pub fn has_disk(&self, device: usize) -> bool {
        self.disks.get(device).and_then(|d| d.as_ref()).is_some()
    }
}
