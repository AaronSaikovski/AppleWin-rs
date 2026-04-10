// ProDOS directory traversal and entry-allocation helpers.
// Ported from ProDOS_BlockGetDirectoryBlockCount, ProDOS_BlockGetPathOffset,
// and ProDOS_DirGetFirstFreeEntryOffset in AppleWin/source/ProDOS_FileSystem.h

use super::types::{BLOCK_SIZE, ProDosKind, ROOT_OFFSET, get_file_header, get_u16};

// ── Directory block helpers ───────────────────────────────────────────────────

/// Count how many 512-byte blocks make up a directory chain starting at `offset`.
/// Traverses the doubly-linked list via the `[2..4]` next-block field.
///
/// Mirrors `ProDOS_BlockGetDirectoryBlockCount`.
#[allow(dead_code)] // available for future directory listing / sub-directory support
pub fn get_dir_block_count(image: &[u8], mut offset: usize) -> usize {
    let mut count = 0;
    loop {
        count += 1;
        let next_block = get_u16(image, offset + 2) as usize;
        if next_block == 0 {
            break;
        }
        offset = next_block * BLOCK_SIZE;
    }
    count
}

/// Return the byte offset in `image` for the given ProDOS path.
/// Only the root path (`"/"` or `""`) is supported; sub-directory navigation
/// is not implemented (matching the C++ behaviour which also left it as TODO).
///
/// Mirrors `ProDOS_BlockGetPathOffset`.
pub fn get_path_offset(path: &str) -> Option<usize> {
    if path.is_empty() || path == "/" {
        Some(ROOT_OFFSET)
    } else {
        // Sub-directory traversal: NOT IMPLEMENTED
        None
    }
}

/// Scan the directory chain starting at `base_offset` for the first entry whose
/// `kind` byte is `PRODOS_KIND_DEL` or whose filename length is zero.
/// Returns the absolute byte offset in `image` of the free entry, or `None` if
/// the directory is full.
///
/// Mirrors `ProDOS_DirGetFirstFreeEntryOffset`.
pub fn get_first_free_entry_offset(
    image: &[u8],
    base_offset: usize,
    entry_len: u8,
    entry_num: u8,
) -> Option<usize> {
    let mut block_off = base_offset;
    loop {
        let mut entry_off = block_off + 4; // skip prev/next pointers

        for _ in 0..entry_num {
            let f = get_file_header(image, entry_off);
            if f.kind == ProDosKind::Del as u8 || f.len == 0 {
                return Some(entry_off);
            }
            entry_off += entry_len as usize;
        }

        let next_block = get_u16(image, block_off + 2) as usize;
        if next_block == 0 {
            break;
        }
        block_off = next_block * BLOCK_SIZE;
    }
    None
}

/// Return the directory block number that contains `base_offset`.
pub fn dir_block_of(base_offset: usize) -> u32 {
    (base_offset / BLOCK_SIZE) as u32
}
