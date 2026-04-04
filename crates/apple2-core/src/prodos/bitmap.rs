// ProDOS volume free-block bitmap operations.
// Ported from ProDOS_BlockInitFree / ProDOS_BlockGetFirstFree / ProDOS_BlockSetUsed
// in AppleWin/source/ProDOS_FileSystem.h

use super::types::{BLOCK_SIZE, MAX_BLOCKS};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Compute the byte offset of the bitmap in the image.
#[inline]
pub fn bitmap_offset(bitmap_block: u16) -> usize {
    bitmap_block as usize * BLOCK_SIZE
}

/// Total number of blocks in a disk image of `disk_size` bytes.
#[inline]
pub fn block_count(disk_size: usize) -> usize {
    disk_size.div_ceil(BLOCK_SIZE)
}

/// Number of bitmap bytes needed to cover all blocks in the image.
#[inline]
fn bitmap_byte_count(disk_size: usize) -> usize {
    block_count(disk_size).div_ceil(8)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Initialize the volume free-bitmap: set all bits to 1 (free) for valid blocks.
/// Updates `total_blocks` in the volume header fields (passed by mutable reference).
/// Returns the number of 512-byte blocks consumed by the bitmap itself.
///
/// Mirrors `ProDOS_BlockInitFree`.
pub fn init_free(image: &mut [u8], disk_size: usize, bitmap_block: u16, total_blocks: &mut u16) {
    let offset = bitmap_offset(bitmap_block);
    let size   = bitmap_byte_count(disk_size);
    for b in &mut image[offset..offset + size] { *b = 0xFF; }

    let blocks = block_count(disk_size);
    *total_blocks = if blocks > MAX_BLOCKS as usize {
        MAX_BLOCKS
    } else {
        blocks as u16
    };
}

/// Return the number of 512-byte bitmap blocks needed for a disk of `disk_size`.
/// Callers use this to know how many bitmap blocks to allocate after calling `init_free`.
pub fn bitmap_block_count(disk_size: usize) -> usize {
    let size = bitmap_byte_count(disk_size);
    size.div_ceil(BLOCK_SIZE)
}

/// Scan the bitmap for the first free (bit = 1) block and return its block number.
/// Returns `None` if the disk is full.
///
/// Mirrors `ProDOS_BlockGetFirstFree`.
pub fn get_first_free(image: &[u8], disk_size: usize, bitmap_block: u16) -> Option<u32> {
    let offset = bitmap_offset(bitmap_block);
    let size   = bitmap_byte_count(disk_size);
    let mut block: u32 = 0;

    for byte in 0..size {
        let mut mask: u8 = 0x80;
        loop {
            if image[offset + byte] & mask != 0 {
                return Some(block);
            }
            mask >>= 1;
            block += 1;
            if mask == 0 { break; }
        }
    }
    None
}

/// Mark block `block` as used (clear its bit in the bitmap).
///
/// Mirrors `ProDOS_BlockSetUsed`.
pub fn set_used(image: &mut [u8], bitmap_block: u16, block: u32) {
    let offset = bitmap_offset(bitmap_block);
    let byte   = (block / 8) as usize;
    let bit    = (block % 8) as u8;
    let mask   = 0x80u8 >> bit;
    // XOR after OR — equivalent to clearing the bit (matches C++ exactly)
    image[offset + byte] |= mask;
    image[offset + byte] ^= mask;
}

