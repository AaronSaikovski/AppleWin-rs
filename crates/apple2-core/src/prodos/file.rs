// ProDOS file allocation: add a file to an existing formatted volume.
// Ported from Util_ProDOS_AddFile in AppleWin/source/ProDOS_Utils.cpp

use super::types::{
    BLOCK_SIZE,
    ProDosKind, FileHeader,
    get_volume_header, set_volume_header,
    put_file_header, put_index_block,
};
use super::bitmap::{get_first_free, set_used};
use super::directory::{get_path_offset, get_first_free_entry_offset, dir_block_of};
use super::types::ProDosError;

// ── File metadata ─────────────────────────────────────────────────────────────

/// Metadata supplied by the caller when adding a file to a ProDOS volume.
/// Mirrors the fields filled in by `Util_ProDOS_CopyBASIC` / `Util_ProDOS_CopyDOS` etc.
#[derive(Debug, Clone)]
pub struct FileMeta {
    /// Filename (ASCII; will be uppercased on write)
    pub name: String,
    /// ProDOS file type byte (0xFF = SYS, 0xFC = BAS, 0x06 = BIN, etc.)
    pub file_type: u8,
    /// Load address (for SYS/BIN files)
    pub aux: u16,
    pub date: u16,
    pub time: u16,
    pub mod_date: u16,
    pub mod_time: u16,
    pub access: u8,
    pub cur_ver: u8,
    pub min_ver: u8,
}

// ── Sparse-block detection ────────────────────────────────────────────────────

/// Return `true` if the 512-byte block at `file_offset` within `file_data` is
/// all zeroes (a "sparse" block that does not need to be written to disk).
///
/// Mirrors `Util_ProDOS_IsFileBlockSparse`.
fn is_sparse(file_offset: usize, file_data: &[u8]) -> bool {
    if file_offset >= file_data.len() {
        return false; // safety guard (dead for valid block indices)
    }
    let end = (file_offset + BLOCK_SIZE).min(file_data.len());
    file_data[file_offset..end].iter().all(|&b| b == 0)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Add a single file to a formatted ProDOS volume image.
///
/// - `image`       — mutable disk image buffer (ProDOS block order)
/// - `disk_size`   — total size of the image in bytes
/// - `file_data`   — raw file content
/// - `meta`        — file metadata (name, type, aux, dates, …)
/// - `allow_sparse`— if true, all-zero blocks are stored as sparse (index entry 0)
///
/// Mirrors `Util_ProDOS_AddFile`.
pub fn add_file(
    image: &mut [u8],
    disk_size: usize,
    file_data: &[u8],
    meta: &FileMeta,
    allow_sparse: bool,
) -> Result<(), ProDosError> {
    // ── Locate root directory ──────────────────────────────────────────────
    let base_offset = get_path_offset("/").unwrap(); // always Some for "/"
    let dir_block   = dir_block_of(base_offset) as u32;

    let mut vh = get_volume_header(image, dir_block);

    // ── Find a free directory entry ────────────────────────────────────────
    let free_entry_off = get_first_free_entry_offset(
        image,
        base_offset,
        vh.entry_len,
        vh.entry_num,
    )
    .ok_or(ProDosError::DiskFull)?;

    // ── Calculate storage kind and block counts ────────────────────────────
    let file_size    = file_data.len();
    let n_data_blks  = (file_size + BLOCK_SIZE - 1) / BLOCK_SIZE;
    let n_data_blks  = n_data_blks.max(1); // at least 1 block even for empty files

    let (kind, n_index_blks) = if file_size <= BLOCK_SIZE {
        (ProDosKind::Seed, 0usize)
    } else if file_size > 256 * BLOCK_SIZE {
        // TREE files: not yet implemented (matches C++ assert)
        return Err(ProDosError::FileTooBig);
    } else {
        // SAPLING: one index block covers up to 256 data blocks (≤ 128 KB)
        (ProDosKind::Sapling, 1usize)
    };

    // ── Allocate index block(s) ────────────────────────────────────────────
    let mut n_blocks_total = n_index_blks as u16;
    let mut inode: u16     = 0; // key block: index block for Sapling, data block for Seed
    let mut index_off: usize = 0; // disk byte offset of the sapling index block

    for i in 0..n_index_blks {
        let blk = get_first_free(image, disk_size, vh.bitmap_block)
            .ok_or(ProDosError::DiskFull)?;
        set_used(image, vh.bitmap_block, blk);

        if i == 0 {
            inode     = blk as u16;
            index_off = blk as usize * BLOCK_SIZE;
        }
        // TREE master-index linking would go here (not implemented)
    }

    // ── Copy data blocks ───────────────────────────────────────────────────
    let slack          = file_size % BLOCK_SIZE;
    let last_blk_size  = if slack != 0 { slack } else { BLOCK_SIZE };

    for i in 0..n_data_blks {
        let data_blk = get_first_free(image, disk_size, vh.bitmap_block)
            .ok_or(ProDosError::DiskFull)?;

        // For Seed, the key block IS the first (and only) data block
        if i == 0 && kind == ProDosKind::Seed {
            inode = data_blk as u16;
        }

        let dst_off    = data_blk as usize * BLOCK_SIZE;
        let src_off    = i * BLOCK_SIZE;
        let is_last    = i == n_data_blks - 1;
        let block_is_sparse = is_sparse(src_off, file_data);

        if block_is_sparse && allow_sparse {
            // Store a zero entry in the sapling index (sparse block)
            if kind == ProDosKind::Sapling && index_off != 0 {
                put_index_block(image, index_off, i, 0);
            }
            // The data block was never written, so don't mark it used.
            // We also do NOT allocate it (we already called get_first_free
            // above but didn't mark it used yet — fix: don't count this block).
            // Note: data_blk was fetched but not yet marked used; skip it.
        } else {
            // Copy the data
            let copy_len = if is_last { last_blk_size } else { BLOCK_SIZE };
            image[dst_off..dst_off + copy_len]
                .copy_from_slice(&file_data[src_off..src_off + copy_len]);

            set_used(image, vh.bitmap_block, data_blk);
            n_blocks_total += 1;

            // Update sapling index
            if kind == ProDosKind::Sapling && index_off != 0 {
                put_index_block(image, index_off, i, data_blk);
            }
        }
    }

    // ── Write the directory entry ──────────────────────────────────────────
    let name_bytes = meta.name.as_bytes();
    let name_len   = name_bytes.len().min(15) as u8;
    let mut name_buf = [0u8; 16];
    for (i, &b) in name_bytes.iter().take(15).enumerate() {
        name_buf[i] = if b.is_ascii_lowercase() { b - b'a' + b'A' } else { b };
    }

    let fh = FileHeader {
        kind:      kind as u8,
        len:       name_len,
        name:      name_buf,
        file_type: meta.file_type,
        inode,
        blocks:    n_blocks_total,
        size:      file_size as u32,
        date:      meta.date,
        time:      meta.time,
        cur_ver:   meta.cur_ver,
        min_ver:   meta.min_ver,
        access:    meta.access,
        aux:       meta.aux,
        mod_date:  meta.mod_date,
        mod_time:  meta.mod_time,
        dir_block: dir_block as u16,
    };
    put_file_header(image, free_entry_off, &fh);

    // ── Update volume header (file_count++) ────────────────────────────────
    vh.file_count += 1;
    set_volume_header(image, &vh, dir_block);

    Ok(())
}
