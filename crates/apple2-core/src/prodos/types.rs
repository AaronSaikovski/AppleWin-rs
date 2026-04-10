// ProDOS on-disk types, constants, and byte-level helpers.
// Ported from AppleWin/source/ProDOS_FileSystem.h

use bitflags::bitflags;
use thiserror::Error;

// ── Constants ─────────────────────────────────────────────────────────────────

pub const BLOCK_SIZE: usize = 0x200; // 512 bytes per block
pub const ROOT_BLOCK: u32 = 2; // Root directory at block 2
pub const ROOT_OFFSET: usize = ROOT_BLOCK as usize * BLOCK_SIZE; // 0x400
pub const MAX_FILENAME: usize = 15;
pub const MAX_BLOCKS: u16 = 0xFFFF; // 32 MB volume limit

// ── ProDOS storage-type codes (high nibble of the kind byte) ──────────────────

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // full set of ProDOS storage types; not all are written yet
pub enum ProDosKind {
    Del = 0x0,     // Deleted / unused entry
    Seed = 0x1,    // Seedling: ≤ 512 bytes, key block IS the data block
    Sapling = 0x2, // Sapling: ≤ 128 KB, one index block
    Tree = 0x3,    // Tree: > 128 KB, master + sub-index blocks
    Pascal = 0x4,
    GsOs = 0x5,
    Dir = 0xD,  // Sub-directory entry in parent
    Sub = 0xE,  // Sub-directory header (points back to parent)
    Root = 0xF, // Volume (root) directory header
}

impl ProDosKind {
    #[allow(dead_code)] // used when reading/parsing disk images
    pub fn from_u8(v: u8) -> Self {
        match v & 0xF {
            0x0 => Self::Del,
            0x1 => Self::Seed,
            0x2 => Self::Sapling,
            0x3 => Self::Tree,
            0x4 => Self::Pascal,
            0x5 => Self::GsOs,
            0xD => Self::Dir,
            0xE => Self::Sub,
            0xF => Self::Root,
            _ => Self::Del,
        }
    }
}

// ── Access flags ──────────────────────────────────────────────────────────────

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Access: u8 {
        const D = 0x80; // Can destroy
        const N = 0x40; // Can rename
        const B = 0x20; // Can backup
        const I = 0x04; // Invisible
        const W = 0x02; // Can write
        const R = 0x01; // Can read
    }
}

// ── On-disk structs ───────────────────────────────────────────────────────────

/// Mirrors ProDOS_VolumeHeader_t.
/// Use `get_volume_header` / `set_volume_header` to read/write from a disk image.
#[derive(Debug, Clone, Default)]
pub struct VolumeHeader {
    pub kind: u8,       // ProDosKind (4 bits)
    pub len: u8,        // Filename length (4 bits)
    pub name: [u8; 16], // Volume name (null-terminated, len bytes used on disk)
    pub pad8: [u8; 8],  // Root-only padding / sub-dir magic byte
    pub date: u16,
    pub time: u16,
    pub cur_ver: u8,
    pub min_ver: u8,
    pub access: u8,
    pub entry_len: u8,   // Size of each directory entry in bytes (0x27)
    pub entry_num: u8,   // Number of directory entries per block (0x0D)
    pub file_count: u16, // Active file entries in this directory
    // Volume (Root) extra fields:
    pub bitmap_block: u16,
    pub total_blocks: u16,
    // Sub-directory extra fields (overlaid with bitmap/total):
    #[allow(dead_code)] // used when reading sub-directory headers
    pub parent_block: u16,
    #[allow(dead_code)]
    pub parent_entry_num: u8,
    #[allow(dead_code)]
    pub parent_entry_len: u8,
}

/// Mirrors ProDOS_FileHeader_t.
/// Use `get_file_header` / `put_file_header` to read/write from a disk image.
#[derive(Debug, Clone, Default)]
pub struct FileHeader {
    pub kind: u8,       // ProDosKind (4 bits)
    pub len: u8,        // Filename length (4 bits)
    pub name: [u8; 16], // Filename (null-terminated, len bytes used on disk)
    pub file_type: u8,  // User-defined type byte (0xFF = SYS, etc.)
    pub inode: u16,     // Key block pointer
    pub blocks: u16,    // Total blocks used (index + non-sparse data blocks)
    pub size: u32,      // EOF address (24-bit on disk)
    pub date: u16,
    pub time: u16,
    pub cur_ver: u8,
    pub min_ver: u8,
    pub access: u8,
    pub aux: u16, // Load address for BIN/SYS files
    pub mod_date: u16,
    pub mod_time: u16,
    pub dir_block: u16, // Pointer back to directory block
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ProDosError {
    #[error("disk full")]
    DiskFull,
    #[error("file too large (TREE files not yet implemented)")]
    FileTooBig,
    #[error("invalid volume or file name")]
    InvalidName,
    #[error("disk image exceeds maximum ProDOS volume size")]
    ImageTooLarge,
    #[error("disk image too small for DOS 3.3")]
    ImageTooSmall,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ── Date / time packing ───────────────────────────────────────────────────────

/// Pack a ProDOS date word.
/// Bit layout: `YYYYYYYMMMMDDDDD`
/// year  = 2-digit year (e.g. 23 = 2023), month = 1-12, day = 1-31
pub fn pack_date(year: u16, month: u8, day: u8) -> u16 {
    ((year & 0x7F) << 9) | (((month as u16) & 0x0F) << 5) | ((day as u16) & 0x1F)
}

/// Pack a ProDOS time word.
/// Bit layout: `000HHHHHH00MMMMMM`
pub fn pack_time(hours: u8, minutes: u8) -> u16 {
    (((hours as u16) & 0x1F) << 8) | ((minutes as u16) & 0x3F)
}

// ── Little-endian byte helpers ────────────────────────────────────────────────

#[inline]
pub fn get_u16(buf: &[u8], off: usize) -> u16 {
    (buf[off] as u16) | ((buf[off + 1] as u16) << 8)
}

#[inline]
pub fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off] = (v & 0xFF) as u8;
    buf[off + 1] = ((v >> 8) & 0xFF) as u8;
}

#[inline]
pub fn get_u24(buf: &[u8], off: usize) -> u32 {
    (buf[off] as u32) | ((buf[off + 1] as u32) << 8) | ((buf[off + 2] as u32) << 16)
}

#[inline]
pub fn put_u24(buf: &mut [u8], off: usize, v: u32) {
    buf[off] = (v & 0xFF) as u8;
    buf[off + 1] = ((v >> 8) & 0xFF) as u8;
    buf[off + 2] = ((v >> 16) & 0xFF) as u8;
}

// ── ProDOS split index-block helpers ─────────────────────────────────────────
// An index block stores 256 lo-bytes at offset 0..255 and 256 hi-bytes at
// offset 256..511, giving 256 block pointers per index block.

#[allow(dead_code)] // used when reading sapling/tree files
#[inline]
pub fn get_index_block(buf: &[u8], block_off: usize, entry: usize) -> u32 {
    let lo = buf[block_off + entry] as u32;
    let hi = buf[block_off + entry + 256] as u32;
    lo | (hi << 8)
}

#[inline]
pub fn put_index_block(buf: &mut [u8], block_off: usize, entry: usize, block: u32) {
    buf[block_off + entry] = (block & 0xFF) as u8;
    buf[block_off + entry + 256] = ((block >> 8) & 0xFF) as u8;
}

// ── Filename helpers ──────────────────────────────────────────────────────────

/// Copy `src` into `dst` as ASCII uppercase, up to `MAX_FILENAME` characters.
/// Returns the number of bytes written (not including the null terminator).
pub fn copy_upper(dst: &mut [u8], src: &str) -> usize {
    let mut n = 0;
    for (i, b) in src.bytes().enumerate() {
        if i >= MAX_FILENAME {
            break;
        }
        let c = if b.is_ascii_lowercase() {
            b - b'a' + b'A'
        } else {
            b
        };
        dst[i] = c;
        n += 1;
    }
    if n < dst.len() {
        dst[n] = 0;
    }
    n
}

// ── Volume header serialization ───────────────────────────────────────────────

/// Read a `VolumeHeader` from `image` at the given block.
/// `base = block * BLOCK_SIZE + 4` (skip prev/next 4 bytes).
pub fn get_volume_header(image: &[u8], block: u32) -> VolumeHeader {
    let base = block as usize * BLOCK_SIZE + 4;
    let kind_len = image[base];
    let len = kind_len & 0xF;
    let mut name = [0u8; MAX_FILENAME + 1];
    for i in 0..(len as usize) {
        name[i] = image[base + 1 + i];
    }
    let mut pad8 = [0u8; 8];
    for i in 0..8 {
        pad8[i] = image[base + 16 + i];
    }
    VolumeHeader {
        kind: (kind_len >> 4) & 0xF,
        len,
        name,
        pad8,
        date: get_u16(image, base + 24),
        time: get_u16(image, base + 26),
        cur_ver: image[base + 28],
        min_ver: image[base + 29],
        access: image[base + 30],
        entry_len: image[base + 31],
        entry_num: image[base + 32],
        file_count: get_u16(image, base + 33),
        bitmap_block: get_u16(image, base + 35),
        total_blocks: get_u16(image, base + 37),
        parent_block: 0,
        parent_entry_num: 0,
        parent_entry_len: 0,
    }
}

/// Write a `VolumeHeader` into `image` at the given block.
pub fn set_volume_header(image: &mut [u8], h: &VolumeHeader, block: u32) {
    let base = block as usize * BLOCK_SIZE + 4;
    image[base] = ((h.kind & 0xF) << 4) | (h.len & 0xF);
    for i in 0..(h.len as usize) {
        image[base + 1 + i] = h.name[i];
    }
    for i in 0..8 {
        image[base + 16 + i] = h.pad8[i];
    }
    put_u16(image, base + 24, h.date);
    put_u16(image, base + 26, h.time);
    image[base + 28] = h.cur_ver;
    image[base + 29] = h.min_ver;
    image[base + 30] = h.access;
    image[base + 31] = h.entry_len;
    image[base + 32] = h.entry_num;
    put_u16(image, base + 33, h.file_count);
    put_u16(image, base + 35, h.bitmap_block);
    put_u16(image, base + 37, h.total_blocks);
}

// ── File header serialization ─────────────────────────────────────────────────

/// Read a `FileHeader` from `image` at byte offset `off`.
pub fn get_file_header(image: &[u8], off: usize) -> FileHeader {
    let kind_len = image[off];
    let len = kind_len & 0xF;
    let mut name = [0u8; MAX_FILENAME + 1];
    for i in 0..(len as usize) {
        name[i] = image[off + 1 + i];
    }
    FileHeader {
        kind: (kind_len >> 4) & 0xF,
        len,
        name,
        file_type: image[off + 16],
        inode: get_u16(image, off + 17),
        blocks: get_u16(image, off + 19),
        size: get_u24(image, off + 21),
        date: get_u16(image, off + 24),
        time: get_u16(image, off + 26),
        cur_ver: image[off + 28],
        min_ver: image[off + 29],
        access: image[off + 30],
        aux: get_u16(image, off + 31),
        mod_date: get_u16(image, off + 33),
        mod_time: get_u16(image, off + 35),
        dir_block: get_u16(image, off + 37),
    }
}

/// Write a `FileHeader` into `image` at byte offset `off`.
pub fn put_file_header(image: &mut [u8], off: usize, f: &FileHeader) {
    image[off] = ((f.kind & 0xF) << 4) | (f.len & 0xF);
    for i in 0..(f.len as usize) {
        image[off + 1 + i] = f.name[i];
    }
    image[off + 16] = f.file_type;
    put_u16(image, off + 17, f.inode);
    put_u16(image, off + 19, f.blocks);
    put_u24(image, off + 21, f.size);
    put_u16(image, off + 24, f.date);
    put_u16(image, off + 26, f.time);
    image[off + 28] = f.cur_ver;
    image[off + 29] = f.min_ver;
    image[off + 30] = f.access;
    put_u16(image, off + 31, f.aux);
    put_u16(image, off + 33, f.mod_date);
    put_u16(image, off + 35, f.mod_time);
    put_u16(image, off + 37, f.dir_block);
}
