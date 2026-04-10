//! Disk image utility functions: decompression (.gz, .zip) and 2IMG wrapper parsing.

use std::io::Read;

// ── Decompression ────────────────────────────────────────────────────────────

/// Attempt to decompress `data` if it looks like gzip or zip.
///
/// Returns `(payload, inner_extension)`.  If the data is not compressed the
/// original bytes and extension are returned unchanged.
pub fn decompress(data: &[u8], ext: &str) -> (Vec<u8>, String) {
    // gzip: magic 0x1F 0x8B
    if data.len() >= 2
        && data[0] == 0x1F
        && data[1] == 0x8B
        && let Ok(decompressed) = decompress_gz(data)
    {
        let inner = strip_gz_ext(ext);
        return (decompressed, inner);
    }

    // zip: magic PK (0x50 0x4B)
    if data.len() >= 4
        && data[0] == 0x50
        && data[1] == 0x4B
        && let Ok((decompressed, inner_name)) = decompress_zip(data)
    {
        let inner = ext_from_filename(&inner_name).unwrap_or_else(|| strip_gz_ext(ext));
        return (decompressed, inner);
    }

    (data.to_vec(), ext.to_string())
}

fn decompress_gz(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

fn decompress_zip(data: &[u8]) -> std::io::Result<(Vec<u8>, String)> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // Extract the first file in the archive.
    let mut file = archive
        .by_index(0)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let name = file.name().to_string();
    let mut out = Vec::new();
    file.read_to_end(&mut out)?;
    Ok((out, name))
}

/// Strip .gz from the extension: "dsk.gz" → "dsk", "nib.gz" → "nib".
fn strip_gz_ext(ext: &str) -> String {
    let lower = ext.to_ascii_lowercase();
    if lower.ends_with(".gz") {
        lower[..lower.len() - 3].to_string()
    } else if lower == "gz" {
        // Bare .gz — caller should look at the filename before the .gz
        "dsk".to_string()
    } else {
        lower
    }
}

/// Extract the file extension from a filename (e.g. "game.dsk" → Some("dsk")).
fn ext_from_filename(name: &str) -> Option<String> {
    let name = name.rsplit('/').next().unwrap_or(name);
    let name = name.rsplit('\\').next().unwrap_or(name);
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())
}

// ── 2IMG wrapper ──────────────────────────���──────────────────────────────────

/// Magic signature for 2IMG files: "2IMG" in ASCII.
const MAGIC_2IMG: [u8; 4] = [0x32, 0x49, 0x4D, 0x47];

/// Image format values in the 2IMG header at offset 0x0C.
const IMG_FORMAT_DOS33: u32 = 0;
const IMG_FORMAT_PRODOS: u32 = 1;
const IMG_FORMAT_NIB: u32 = 2;

/// Attempt to parse a 2IMG wrapper header.
///
/// Returns `Some((payload, format_ext))` where `format_ext` is "dsk", "po",
/// or "nib" depending on the format field.  Returns `None` if the data does
/// not have a valid 2IMG header.
pub fn unwrap_2img(data: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    if data.len() < 64 {
        return None;
    }
    if data[0..4] != MAGIC_2IMG {
        return None;
    }

    // Header size at offset 0x08 (little-endian u16).
    let header_size = u16::from_le_bytes([data[8], data[9]]) as usize;
    if header_size < 64 || header_size > data.len() {
        return None;
    }

    // Image data length at offset 0x1C (little-endian u32).
    let data_len = u32::from_le_bytes([data[0x1C], data[0x1D], data[0x1E], data[0x1F]]) as usize;
    if header_size + data_len > data.len() {
        return None;
    }

    // Image format at offset 0x0C (little-endian u32).
    let format = u32::from_le_bytes([data[0x0C], data[0x0D], data[0x0E], data[0x0F]]);

    let ext = match format {
        IMG_FORMAT_DOS33 => "dsk",
        IMG_FORMAT_PRODOS => "po",
        IMG_FORMAT_NIB => "nib",
        _ => return None,
    };

    let payload = data[header_size..header_size + data_len].to_vec();
    Some((payload, ext))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompress_passthrough() {
        let data = vec![0x01, 0x02, 0x03];
        let (out, ext) = decompress(&data, "dsk");
        assert_eq!(out, data);
        assert_eq!(ext, "dsk");
    }

    #[test]
    fn decompress_gz() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;

        let original = b"Hello Apple II disk image data!";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let (out, ext) = decompress(&compressed, "dsk.gz");
        assert_eq!(out, original);
        assert_eq!(ext, "dsk");
    }

    #[test]
    fn unwrap_2img_valid() {
        // Build a minimal 2IMG header + payload.
        let payload = vec![0xAA; 256];
        let mut header = vec![0u8; 64];
        header[0..4].copy_from_slice(&MAGIC_2IMG);
        // Header size = 64 (at offset 0x08, le u16).
        header[8] = 64;
        header[9] = 0;
        // Format = ProDOS (1) at offset 0x0C.
        header[0x0C] = 1;
        // Data length at offset 0x1C.
        let len_bytes = (payload.len() as u32).to_le_bytes();
        header[0x1C..0x20].copy_from_slice(&len_bytes);

        let mut data = header;
        data.extend_from_slice(&payload);

        let (out, ext) = unwrap_2img(&data).unwrap();
        assert_eq!(out, payload);
        assert_eq!(ext, "po");
    }

    #[test]
    fn unwrap_2img_too_short() {
        assert!(unwrap_2img(&[0; 32]).is_none());
    }

    #[test]
    fn unwrap_2img_bad_magic() {
        let data = vec![0u8; 64];
        assert!(unwrap_2img(&data).is_none());
    }

    #[test]
    fn strip_gz_ext_tests() {
        assert_eq!(strip_gz_ext("dsk.gz"), "dsk");
        assert_eq!(strip_gz_ext("nib.gz"), "nib");
        assert_eq!(strip_gz_ext("gz"), "dsk");
        assert_eq!(strip_gz_ext("po"), "po");
    }
}
