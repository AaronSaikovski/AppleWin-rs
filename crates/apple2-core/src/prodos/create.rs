// High-level ProDOS (and DOS 3.3) disk-image creation API.
// Ported from New_DOSProDOS_Disk, New_Blank_Disk, Format_ProDOS_Disk,
// Format_DOS33_Disk in AppleWin/source/ProDOS_Utils.cpp

use std::path::Path;
use std::io::Write as _;

use super::types::{
    Access, ProDosError,
    pack_date, pack_time,
};
use super::file::{add_file, FileMeta};
use super::format::{
    format_filesystem, format_dos33_filesystem,
    forward_sector_interleave, reverse_sector_interleave,
};

// ── Embedded firmware binaries ────────────────────────────────────────────────

const PRODOS_BIN:     &[u8] = include_bytes!("../../roms/firmware/prodos243.bin");
const BOOT_BIN:       &[u8] = include_bytes!("../../roms/firmware/bootsector_prodos243.bin");
const BASIC_BIN:      &[u8] = include_bytes!("../../roms/firmware/basic17.system.bin");
const BITSY_BOOT_BIN: &[u8] = include_bytes!("../../roms/firmware/bitsy.boot.bin");
const QUIT_BIN:       &[u8] = include_bytes!("../../roms/firmware/quit.system.bin");
const DOS33_BIN:      &[u8] = include_bytes!("../../roms/firmware/dos33c.bin");

// ── DOS 3.3 disk size constraints ─────────────────────────────────────────────

const TRACK_DENIBBLIZED_SIZE: usize = 16 * 256;
const MIN_DOS33_SIZE: usize = 34 * TRACK_DENIBBLIZED_SIZE;     // ~34 tracks minimum
const MAX_DOS33_SIZE: usize = 40 * TRACK_DENIBBLIZED_SIZE;     // 40 tracks maximum
const MAX_PRODOS_SIZE: usize = 512 * 65536;                    // 32 MB

// ── Options struct ────────────────────────────────────────────────────────────

/// Options for creating a new ProDOS disk image.
#[derive(Debug, Clone)]
pub struct ProDosCreateOptions {
    /// ProDOS volume name (will be uppercased; leading '/' stripped if present)
    pub volume_name: String,
    /// Embed BITSY.BOOT (a small bootloader)
    pub copy_bitsy_boot: bool,
    /// Embed QUIT.SYSTEM
    pub copy_bitsy_bye: bool,
    /// Embed BASIC.SYSTEM 1.7
    pub copy_basic: bool,
    /// Embed ProDOS 2.4.3
    pub copy_prodos: bool,
}

impl Default for ProDosCreateOptions {
    fn default() -> Self {
        Self {
            volume_name:   "BLANK".to_string(),
            copy_bitsy_boot: false,
            copy_bitsy_bye:  false,
            copy_basic:      false,
            copy_prodos:     false,
        }
    }
}

// ── File-metadata helpers ─────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn sys_meta(name: &str, aux: u16, cur_ver: u8, min_ver: u8,
            cdate: u16, ctime: u16, mdate: u16, mtime: u16) -> FileMeta {
    let access = (Access::B | Access::R).bits();
    FileMeta {
        name:      name.to_string(),
        file_type: 0xFF, // SYS
        aux,
        date:      cdate,
        time:      ctime,
        mod_date:  mdate,
        mod_time:  mtime,
        access,
        cur_ver,
        min_ver,
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Create a new bootable ProDOS disk image at `path`.
///
/// `size` must be the desired image size in bytes (e.g. 143360 for 140 KB
/// floppy, or a multiple of 512 for a hard-disk image).
///
/// Mirrors `New_DOSProDOS_Disk` (ProDOS path).
pub fn create_prodos_disk(
    path: &Path,
    size: usize,
    opts: &ProDosCreateOptions,
) -> Result<(), ProDosError> {
    let mut image = vec![0u8; size];

    // For a new ProDOS disk we always use ProDOS (block) order — no interleave.
    // The sector interleave is only relevant when re-formatting an existing
    // .dsk image (see `format_prodos_disk`).
    let vol = opts.volume_name.trim_start_matches('/');
    format_filesystem(&mut image, size, vol);

    // Write boot blocks 0–1
    let boot_size = BOOT_BIN.len().min(1024);
    image[..boot_size].copy_from_slice(&BOOT_BIN[..boot_size]);

    // Embed system files in the order the C++ does it
    if opts.copy_bitsy_boot { copy_bitsy_boot(&mut image, size)?; }
    if opts.copy_bitsy_bye  { copy_bitsy_bye (&mut image, size)?; }
    if opts.copy_basic      { copy_basic      (&mut image, size)?; }
    if opts.copy_prodos     { copy_prodos     (&mut image, size)?; }

    write_image(path, &image)
}

/// Create a new bootable DOS 3.3 disk image at `path`.
///
/// Mirrors `New_DOSProDOS_Disk` (DOS 3.3 path).
pub fn create_dos33_disk(path: &Path, size: usize) -> Result<(), ProDosError> {
    if size < MIN_DOS33_SIZE { return Err(ProDosError::ImageTooSmall); }
    if size > MAX_DOS33_SIZE { return Err(ProDosError::ImageTooLarge); }

    let mut image = vec![0u8; size];

    const VTOC_TRACK: usize = 0x11;
    format_dos33_filesystem(&mut image, size, VTOC_TRACK);

    // Copy DOS 3.3 boot tracks (tracks 0-2)
    let dos33_size = (3 * 16 * 256).min(DOS33_BIN.len());
    image[..dos33_size].copy_from_slice(&DOS33_BIN[..dos33_size]);

    // Tracks 1 and 2 must also be marked used in the VTOC
    // (track 0 is already marked by format_dos33_filesystem)
    mark_dos33_tracks_used(&mut image, VTOC_TRACK, &[1, 2]);

    write_image(path, &image)
}

/// Create a blank (zero-filled) disk image with a minimal boot sector.
///
/// Mirrors `New_Blank_Disk`.
pub fn create_blank_disk(path: &Path, size: usize) -> Result<(), ProDosError> {
    let image = vec![0u8; size];
    write_image(path, &image)
}

/// Re-format an existing ProDOS image at `path` (reads, formats, writes back).
/// The sector order is inferred from the file extension.
///
/// Mirrors `Format_ProDOS_Disk`.
pub fn format_prodos_disk(path: &Path) -> Result<(), ProDosError> {
    let mut image = std::fs::read(path)?;
    let size = image.len();

    if size > MAX_PRODOS_SIZE { return Err(ProDosError::ImageTooLarge); }

    let use_interleave = needs_interleave(path);
    if use_interleave { forward_sector_interleave(&mut image, size); }
    format_filesystem(&mut image, size, "BLANK");
    if use_interleave { reverse_sector_interleave(&mut image, size); }

    write_image(path, &image)
}

/// Re-format an existing DOS 3.3 image at `path` (reads, formats, writes back).
///
/// Mirrors `Format_DOS33_Disk`.
pub fn format_dos33_disk(path: &Path) -> Result<(), ProDosError> {
    let mut image = std::fs::read(path)?;
    let size = image.len();

    if size < MIN_DOS33_SIZE { return Err(ProDosError::ImageTooSmall); }
    if size > MAX_DOS33_SIZE { return Err(ProDosError::ImageTooLarge); }

    const VTOC_TRACK: usize = 0x11;
    format_dos33_filesystem(&mut image, size, VTOC_TRACK);
    write_image(path, &image)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn write_image(path: &Path, image: &[u8]) -> Result<(), ProDosError> {
    let mut f = std::fs::File::create(path)?;
    f.write_all(image)?;
    Ok(())
}

/// Return `true` if the file extension indicates DOS 3.3 sector order,
/// which requires interleave swizzling before/after ProDOS formatting.
fn needs_interleave(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("dsk") | Some("do")
    )
}

/// Mark additional tracks as used in the DOS 3.3 VTOC.
fn mark_dos33_tracks_used(image: &mut [u8], vtoc_track: usize, tracks: &[usize]) {
    const DSK: usize = 16 * 256;
    let vtoc_off = vtoc_track * DSK;
    for &t in tracks {
        let off = vtoc_off + 0x38 + t * 4;
        image[off]     = 0x00;
        image[off + 1] = 0x00;
    }
}

// ── File-copy helpers (mirrors Util_ProDOS_Copy* functions) ───────────────────

fn copy_prodos(image: &mut [u8], disk_size: usize) -> Result<(), ProDosError> {
    // PRODOS: 17,128 bytes, SAPLING, cur_ver=0, min_ver=0x80 (ProDOS 2.4.x magic)
    let meta = FileMeta {
        name:      "PRODOS".to_string(),
        file_type: 0xFF,
        aux:       0x0000,
        date:      pack_date(23, 12, 30),
        time:      pack_time(2, 43),
        mod_date:  pack_date(23, 12, 30),
        mod_time:  pack_time(2, 43),
        access:    (Access::D | Access::N | Access::B | Access::W | Access::R).bits(),
        cur_ver:   0x00,
        min_ver:   0x80,
    };
    add_file(image, disk_size, PRODOS_BIN, &meta, true)
}

fn copy_bitsy_boot(image: &mut [u8], disk_size: usize) -> Result<(), ProDosError> {
    let meta = sys_meta(
        "BITSY.BOOT", 0x2000, 0x24, 0x00,
        pack_date(18, 1, 13), pack_time(9, 9),
        pack_date(16, 9, 15), pack_time(9, 49),
    );
    add_file(image, disk_size, BITSY_BOOT_BIN, &meta, false)
}

fn copy_bitsy_bye(image: &mut [u8], disk_size: usize) -> Result<(), ProDosError> {
    let meta = sys_meta(
        "QUIT.SYSTEM", 0x2000, 0x24, 0x00,
        pack_date(18, 1, 13), pack_time(9, 9),
        pack_date(16, 9, 15), pack_time(9, 41),
    );
    add_file(image, disk_size, QUIT_BIN, &meta, false)
}

fn copy_basic(image: &mut [u8], disk_size: usize) -> Result<(), ProDosError> {
    let meta = sys_meta(
        "BASIC.SYSTEM", 0x2000, 0x24, 0x00,
        pack_date(18, 1, 13), pack_time(9, 9),
        pack_date(16, 8, 30), pack_time(7, 56),
    );
    add_file(image, disk_size, BASIC_BIN, &meta, true)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    const FLOPPY_SIZE: usize = 143360; // 35 tracks × 16 sectors × 256 bytes

    #[test]
    fn test_create_blank_disk() {
        let f = NamedTempFile::new().unwrap();
        create_blank_disk(f.path(), FLOPPY_SIZE).unwrap();
        let image = std::fs::read(f.path()).unwrap();
        assert_eq!(image.len(), FLOPPY_SIZE);
    }

    #[test]
    fn test_create_prodos_floppy_volume_header() {
        let f = NamedTempFile::new().unwrap();
        let opts = ProDosCreateOptions {
            volume_name: "TESTDISK".into(),
            ..Default::default()
        };
        create_prodos_disk(f.path(), FLOPPY_SIZE, &opts).unwrap();

        let image = std::fs::read(f.path()).unwrap();
        assert_eq!(image.len(), FLOPPY_SIZE);

        // Volume header at block 2, offset +4 (skips prev/next 4 bytes)
        let base = 2 * 512 + 4;
        let kind_byte = image[base];
        assert_eq!((kind_byte >> 4) & 0xF, 0xF, "Root kind byte expected");
        assert_eq!(kind_byte & 0xF, 8,   "Name length = 8 (TESTDISK)");

        // Volume name bytes 1..9 should be "TESTDISK"
        let name = &image[base + 1..base + 9];
        assert_eq!(name, b"TESTDISK");
    }

    #[test]
    fn test_create_prodos_floppy_with_prodos_file() {
        let f = NamedTempFile::new().unwrap();
        let opts = ProDosCreateOptions {
            volume_name: "MYVOLUME".into(),
            copy_prodos: true,
            ..Default::default()
        };
        create_prodos_disk(f.path(), FLOPPY_SIZE, &opts).unwrap();

        let image = std::fs::read(f.path()).unwrap();
        assert_eq!(image.len(), FLOPPY_SIZE);

        // First file entry starts at block 2 offset +4+39 = base+43
        let base = 2 * 512 + 4;
        let entry = base + 39; // first file entry (after volume header)
        let kind_byte = image[entry];
        // Must be SAPLING (0x2) or SEED (0x1), not DEL (0x0)
        let kind = (kind_byte >> 4) & 0xF;
        assert!(kind == 0x1 || kind == 0x2, "Expected SEED or SAPLING file, got kind={kind}");
    }

    #[test]
    fn test_format_prodos_disk() {
        // Create blank, then re-format as ProDOS
        let f = NamedTempFile::new().unwrap();
        create_blank_disk(f.path(), FLOPPY_SIZE).unwrap();
        format_prodos_disk(f.path()).unwrap();

        let image = std::fs::read(f.path()).unwrap();
        let base = 2 * 512 + 4;
        let kind_byte = image[base];
        assert_eq!((kind_byte >> 4) & 0xF, 0xF, "Root kind expected after format");
    }

    #[test]
    fn test_create_dos33_disk() {
        let f = NamedTempFile::new().unwrap();
        create_dos33_disk(f.path(), FLOPPY_SIZE).unwrap();

        let image = std::fs::read(f.path()).unwrap();
        assert_eq!(image.len(), FLOPPY_SIZE);

        // VTOC at track 17, sector 0 (offset = 17 * 4096)
        let vtoc_off = 17 * 16 * 256;
        assert_eq!(image[vtoc_off + 0x03], 0x03, "DOS 3.3 version byte at VTOC+3");
        assert_eq!(image[vtoc_off + 0x35], 16,   "16 sectors/track");
    }
}
