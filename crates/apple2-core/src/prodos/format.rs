// ProDOS (and DOS 3.3) filesystem formatting, plus sector interleave swizzling.
// Ported from Util_ProDOS_FormatFileSystem, Util_DOS33_FormatFileSystem,
// Util_ProDOS_ForwardSectorInterleave, Util_ProDOS_ReverseSectorInterleave
// in AppleWin/source/ProDOS_Utils.cpp

use super::bitmap::{bitmap_block_count, get_first_free, init_free, set_used};
use super::types::{
    Access, BLOCK_SIZE, ProDosKind, ROOT_BLOCK, VolumeHeader, copy_upper, put_u16,
    set_volume_header,
};

// ── DOS 3.3 constants ─────────────────────────────────────────────────────────

const TRACK_DENIBBLIZED_SIZE: usize = 16 * 256; // 4096 bytes per track
const TRACKS_MAX: usize = 40; // 35 standard + 5 extra (160 KB max)
const DSK_SECTOR_SIZE: usize = 256;
const DEFAULT_VOLUME_NUMBER: u8 = 254;

/// Physical sector order for DOS 3.3 <-> ProDOS interleave swizzle.
/// Index = logical sector → value = physical sector.
const INTERLEAVE_DSK: [usize; 16] = [
    0x0, 0xE, 0xD, 0xC, 0xB, 0xA, 0x9, 0x8, 0x7, 0x6, 0x5, 0x4, 0x3, 0x2, 0x1, 0xF,
];

// ── ProDOS filesystem formatting ──────────────────────────────────────────────

/// Initialize a ProDOS volume on `image`.
///
/// - Clears all bytes from block 2 onward (boots blocks 0–1 are preserved).
/// - Creates 4 doubly-linked root-directory blocks.
/// - Initialises the volume-free bitmap and marks used blocks.
/// - Writes the root volume header.
///
/// Mirrors `Util_ProDOS_FormatFileSystem`.
/// `image` MUST already be in ProDOS (block) order.
pub fn format_filesystem(image: &mut [u8], disk_size: usize, volume_name: &str) {
    // Clear everything from block 2 onward, preserving both boot blocks (0 and 1).
    // Mirrors Util_ProDOS_FormatFileSystem which clears from byte 0x400 = ROOT_OFFSET.
    let boot_size = ROOT_BLOCK as usize * BLOCK_SIZE; // = 0x400 = 1024
    for b in &mut image[boot_size..disk_size] {
        *b = 0;
    }

    const N_ROOT_DIR_BLOCKS: usize = 4;

    // Bitmap lives immediately after the root directory blocks
    let bitmap_block = (ROOT_BLOCK as usize + N_ROOT_DIR_BLOCKS) as u16;

    // Init bitmap (all blocks free) and get total_blocks
    let mut total_blocks: u16 = 0;
    init_free(image, disk_size, bitmap_block, &mut total_blocks);

    // Mark boot blocks (0 and 1) as used
    for blk in 0..ROOT_BLOCK {
        set_used(image, bitmap_block, blk);
    }

    // Allocate root directory blocks (blocks 2-5 on a freshly zeroed image)
    let mut prev_dir_block: u16 = 0;
    for i in 0..N_ROOT_DIR_BLOCKS {
        let blk = get_first_free(image, disk_size, bitmap_block)
            .expect("should have free blocks on a freshly formatted image");
        set_used(image, bitmap_block, blk);
        let blk = blk as u16;

        let off = blk as usize * BLOCK_SIZE;
        // Write prev/next pointers: prev = previous block, next = 0 (filled in later)
        put_u16(image, off, prev_dir_block);
        put_u16(image, off + 2, 0);

        if i > 0 {
            // Fix up previous block's next pointer to point here
            let prev_off = prev_dir_block as usize * BLOCK_SIZE;
            put_u16(image, prev_off + 2, blk);
        }

        prev_dir_block = blk;
    }

    // Allocate bitmap blocks
    let n_bitmap_blks = bitmap_block_count(disk_size);
    for _ in 0..n_bitmap_blks {
        let blk = get_first_free(image, disk_size, bitmap_block)
            .expect("should have free blocks for bitmap");
        set_used(image, bitmap_block, blk);
    }

    // Build and write volume header
    let mut vh = VolumeHeader {
        kind: ProDosKind::Root as u8,
        entry_len: 0x27,                      // 39 bytes per entry
        entry_num: (BLOCK_SIZE / 0x27) as u8, // 13
        file_count: 0,
        bitmap_block,
        total_blocks,
        access: (Access::D | Access::N | Access::B | Access::W | Access::R).bits(),
        ..VolumeHeader::default()
    };

    // Strip leading '/' and uppercase the volume name
    let vname = volume_name.trim_start_matches('/');
    let name_len = copy_upper(&mut vh.name, vname);
    vh.len = name_len as u8;

    set_volume_header(image, &vh, ROOT_BLOCK);
}

// ── DOS 3.3 filesystem formatting ─────────────────────────────────────────────

/// Set the usage bits for a single track in the VTOC.
/// `bitmask`: 1 = free, 0 = used.
///
/// Mirrors `Util_DOS33_SetTrackSectorUsage`.
fn dos33_set_track_sector_usage(vtoc: &mut [u8], track: usize, bitmask: u16) {
    let off = 0x38 + track * 4;
    vtoc[off] = ((bitmask >> 8) & 0xFF) as u8;
    vtoc[off + 1] = (bitmask & 0xFF) as u8;
    vtoc[off + 2] = 0x00;
    vtoc[off + 3] = 0x00;
}

/// Write a DOS 3.3 VTOC + catalog chain into `image`.
/// `vtoc_track` is normally 0x11 (track 17).
///
/// Mirrors `Util_DOS33_FormatFileSystem`.
pub fn format_dos33_filesystem(image: &mut [u8], disk_size: usize, vtoc_track: usize) {
    let n_tracks = disk_size / TRACK_DENIBBLIZED_SIZE;
    assert!(n_tracks <= TRACKS_MAX);

    // Write the catalog chain (sector $F down to $2, each pointing to next)
    for sector in (2..=0xF_usize).rev() {
        let off = vtoc_track * TRACK_DENIBBLIZED_SIZE + sector * DSK_SECTOR_SIZE;
        image[off + 1] = vtoc_track as u8;
        image[off + 2] = (sector - 1) as u8;
    }

    // Last catalog sector has no link (sector $1)
    let off = vtoc_track * TRACK_DENIBBLIZED_SIZE + DSK_SECTOR_SIZE;
    image[off + 1] = 0;
    image[off + 2] = 0;

    // FTOC entries per sector: (256 - 12) / 2 = 122
    const FTOC_ENTRIES: u8 = 122;

    let vtoc_off = vtoc_track * TRACK_DENIBBLIZED_SIZE;
    image[vtoc_off + 0x01] = vtoc_track as u8; // catalog track
    image[vtoc_off + 0x02] = 0x0F; // catalog sector
    image[vtoc_off + 0x03] = 0x03; // DOS 3.3
    image[vtoc_off + 0x06] = DEFAULT_VOLUME_NUMBER;
    image[vtoc_off + 0x27] = FTOC_ENTRIES;
    image[vtoc_off + 0x30] = vtoc_track as u8; // last track allocated
    image[vtoc_off + 0x31] = 1; // direction = +1
    image[vtoc_off + 0x34] = n_tracks as u8;
    image[vtoc_off + 0x35] = 16; // sectors/track
    image[vtoc_off + 0x36] = 0x00; // 256 bytes/sector lo
    image[vtoc_off + 0x37] = 0x01; // 256 bytes/sector hi

    // Set track usage bitmap for all tracks
    let mut vtoc_block = vec![0u8; TRACK_DENIBBLIZED_SIZE];
    let src = &image[vtoc_off..vtoc_off + TRACK_DENIBBLIZED_SIZE];
    vtoc_block.copy_from_slice(src);

    for track in 0..n_tracks {
        let bitmask: u16 = if track == 0 || track == vtoc_track {
            0x0000 // track 0 and VTOC track are always fully used
        } else {
            0xFFFF // all other tracks are free
        };
        dos33_set_track_sector_usage(&mut vtoc_block, track, bitmask);
    }

    image[vtoc_off..vtoc_off + TRACK_DENIBBLIZED_SIZE].copy_from_slice(&vtoc_block);
}

// ── Sector interleave swizzling ───────────────────────────────────────────────

/// Re-order 256-byte sectors from DOS 3.3 physical order to ProDOS linear order.
/// Only applies to 35-track floppy images (143360 bytes = 35 * 4096).
/// For hard disk / `.po` images this is a no-op.
///
/// Mirrors `Util_ProDOS_ForwardSectorInterleave` with `INTERLEAVE_DOS33_ORDER`.
pub fn forward_sector_interleave(image: &mut [u8], disk_size: usize) {
    let n_tracks = disk_size / TRACK_DENIBBLIZED_SIZE;
    if !disk_size.is_multiple_of(TRACK_DENIBBLIZED_SIZE) || n_tracks == 0 {
        return;
    }

    let source = image[..disk_size].to_vec();
    let mut offset = 0;

    for _ in 0..n_tracks {
        for (sector, &interleaved) in INTERLEAVE_DSK.iter().enumerate() {
            let src = interleaved * DSK_SECTOR_SIZE;
            let dst = sector * DSK_SECTOR_SIZE;
            image[offset + dst..offset + dst + DSK_SECTOR_SIZE]
                .copy_from_slice(&source[offset + src..offset + src + DSK_SECTOR_SIZE]);
        }
        offset += TRACK_DENIBBLIZED_SIZE;
    }
}

/// Re-order 256-byte sectors from ProDOS linear order back to DOS 3.3 physical order.
/// Only applies to 35-track floppy images.
///
/// Mirrors `Util_ProDOS_ReverseSectorInterleave` with `INTERLEAVE_DOS33_ORDER`.
pub fn reverse_sector_interleave(image: &mut [u8], disk_size: usize) {
    let n_tracks = disk_size / TRACK_DENIBBLIZED_SIZE;
    if !disk_size.is_multiple_of(TRACK_DENIBBLIZED_SIZE) || n_tracks == 0 {
        return;
    }

    let source = image[..disk_size].to_vec();
    let mut offset = 0;

    for _ in 0..n_tracks {
        for (sector, &interleaved) in INTERLEAVE_DSK.iter().enumerate() {
            let src = sector * DSK_SECTOR_SIZE;
            let dst = interleaved * DSK_SECTOR_SIZE;
            image[offset + dst..offset + dst + DSK_SECTOR_SIZE]
                .copy_from_slice(&source[offset + src..offset + src + DSK_SECTOR_SIZE]);
        }
        offset += TRACK_DENIBBLIZED_SIZE;
    }
}
