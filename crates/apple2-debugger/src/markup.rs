//! Data / code markup for the disassembler.
//!
//! Reference: `source/Debugger/Debugger_DisassemblerData.cpp`
//!
//! Allows marking address ranges as data (bytes, words, ASCII strings)
//! rather than code, so the disassembly view renders them appropriately.

use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

/// How a marked region should be displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkupKind {
    /// Disassemble as code (default).
    Code,
    /// Show as single bytes: `DB $XX`.
    Bytes,
    /// Show as 16-bit words (little-endian): `DW $XXXX`.
    Words,
    /// Show as ASCII text: `ASC "..."`.
    Ascii,
    /// Show as address pointers (16-bit, little-endian): `DA $XXXX`.
    Addresses,
}

/// A single markup region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkupRegion {
    pub start: u16,
    pub length: u16,
    pub kind: MarkupKind,
    pub label: Option<String>,
}

/// Manages markup regions for the disassembly view.
///
/// Uses a BTreeMap keyed by start address for efficient lookup.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MarkupMap {
    /// Regions keyed by start address.
    regions: BTreeMap<u16, MarkupRegion>,
}

impl MarkupMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a markup region.
    pub fn add(&mut self, region: MarkupRegion) {
        self.regions.insert(region.start, region);
    }

    /// Remove the markup region starting at `addr`.
    pub fn remove(&mut self, addr: u16) {
        self.regions.remove(&addr);
    }

    /// Mark a range as a specific kind.
    pub fn mark(&mut self, start: u16, length: u16, kind: MarkupKind) {
        self.add(MarkupRegion {
            start,
            length,
            kind,
            label: None,
        });
    }

    /// Get the markup kind for an address, if it falls within a marked region.
    pub fn kind_at(&self, addr: u16) -> Option<MarkupKind> {
        // Check all regions that start at or before `addr`.
        for (&start, region) in self.regions.range(..=addr).rev() {
            let end = start.wrapping_add(region.length);
            if addr >= start && addr < end {
                return Some(region.kind);
            }
            // Since BTreeMap is sorted, once we pass a region that can't contain addr, stop.
            if start < addr.saturating_sub(0xFFFF) {
                break;
            }
        }
        None
    }

    /// Get the markup region containing `addr`, if any.
    pub fn region_at(&self, addr: u16) -> Option<&MarkupRegion> {
        if let Some((&start, region)) = self.regions.range(..=addr).next_back() {
            let end = start.wrapping_add(region.length);
            if addr >= start && addr < end {
                return Some(region);
            }
        }
        None
    }

    /// Iterate over all markup regions.
    pub fn iter(&self) -> impl Iterator<Item = &MarkupRegion> {
        self.regions.values()
    }

    /// Clear all markup.
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Return the number of marked regions.
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    /// Return true if no regions are marked.
    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}

/// Format a data region for display in the disassembly view.
///
/// Returns lines of formatted data and the number of bytes consumed.
pub fn format_data_region<F>(addr: u16, kind: MarkupKind, remaining: u16, mut read: F) -> (Vec<String>, u16)
where
    F: FnMut(u16) -> u8,
{
    let mut lines = Vec::new();
    let mut consumed = 0u16;
    let max = remaining.min(16); // limit per call to keep display manageable

    match kind {
        MarkupKind::Code => {
            // Shouldn't be called for code, but handle gracefully
            return (lines, 0);
        }
        MarkupKind::Bytes => {
            let mut hex = String::new();
            for i in 0..max {
                let b = read(addr.wrapping_add(i));
                hex.push_str(&format!("${:02X} ", b));
                consumed += 1;
            }
            lines.push(format!("{:04X}: DB {}", addr, hex.trim_end()));
        }
        MarkupKind::Words => {
            let count = (max / 2).max(1);
            let mut parts = String::new();
            for i in 0..count {
                let off = i * 2;
                if off + 1 >= remaining { break; }
                let lo = read(addr.wrapping_add(off)) as u16;
                let hi = read(addr.wrapping_add(off + 1)) as u16;
                let word = (hi << 8) | lo;
                parts.push_str(&format!("${:04X} ", word));
                consumed += 2;
            }
            if consumed == 0 {
                let b = read(addr);
                lines.push(format!("{:04X}: DB ${:02X}", addr, b));
                consumed = 1;
            } else {
                lines.push(format!("{:04X}: DW {}", addr, parts.trim_end()));
            }
        }
        MarkupKind::Ascii => {
            let mut text = String::new();
            for i in 0..max {
                let b = read(addr.wrapping_add(i));
                // Apple II high-bit ASCII: strip bit 7 for display
                let ch = b & 0x7F;
                if (0x20..=0x7E).contains(&ch) {
                    text.push(ch as char);
                } else {
                    text.push('.');
                }
                consumed += 1;
            }
            lines.push(format!("{:04X}: ASC \"{}\"", addr, text));
        }
        MarkupKind::Addresses => {
            let count = (max / 2).max(1);
            let mut parts = String::new();
            for i in 0..count {
                let off = i * 2;
                if off + 1 >= remaining { break; }
                let lo = read(addr.wrapping_add(off)) as u16;
                let hi = read(addr.wrapping_add(off + 1)) as u16;
                let word = (hi << 8) | lo;
                parts.push_str(&format!("${:04X} ", word));
                consumed += 2;
            }
            if consumed == 0 {
                consumed = 1;
                lines.push(format!("{:04X}: DB ${:02X}", addr, read(addr)));
            } else {
                lines.push(format!("{:04X}: DA {}", addr, parts.trim_end()));
            }
        }
    }

    (lines, consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_lookup() {
        let mut map = MarkupMap::new();
        map.mark(0x0300, 0x10, MarkupKind::Bytes);
        assert_eq!(map.kind_at(0x0300), Some(MarkupKind::Bytes));
        assert_eq!(map.kind_at(0x030F), Some(MarkupKind::Bytes));
        assert_eq!(map.kind_at(0x0310), None);
        assert_eq!(map.kind_at(0x02FF), None);
    }

    #[test]
    fn format_bytes() {
        let (lines, consumed) = format_data_region(0x0300, MarkupKind::Bytes, 4, |a| (a & 0xFF) as u8);
        assert_eq!(consumed, 4);
        assert!(lines[0].contains("DB"));
    }

    #[test]
    fn format_ascii() {
        let data = b"HELLO";
        let (lines, consumed) = format_data_region(0x0300, MarkupKind::Ascii, 5, |a| {
            data[(a - 0x0300) as usize]
        });
        assert_eq!(consumed, 5);
        assert!(lines[0].contains("HELLO"));
    }
}
