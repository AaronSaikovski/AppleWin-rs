//! Soft-switch display helpers.
//!
//! Formats the current memory mode flags as human-readable text
//! for the debugger's soft-switch panel.

/// Decoded soft-switch state for display.
#[derive(Debug, Default)]
pub struct SoftSwitchInfo {
    pub items: Vec<SwitchEntry>,
}

#[derive(Debug)]
pub struct SwitchEntry {
    pub name:    &'static str,
    pub active:  bool,
    pub address: &'static str,
}

/// Decode memory mode flags into displayable soft-switch entries.
///
/// `mode_bits` is the raw `MemMode` bitfield value.
pub fn decode_soft_switches(mode_bits: u32) -> SoftSwitchInfo {
    let has = |bit: u32| mode_bits & bit != 0;

    let items = vec![
        SwitchEntry { name: "80STORE",   active: has(0x0001), address: "$C000/$C001" },
        SwitchEntry { name: "ALTZP",     active: has(0x0002), address: "$C008/$C009" },
        SwitchEntry { name: "RAMRD",     active: has(0x0004), address: "$C002/$C003" },
        SwitchEntry { name: "RAMWRT",    active: has(0x0008), address: "$C004/$C005" },
        SwitchEntry { name: "BANK2",     active: has(0x0010), address: "$C080-$C08F" },
        SwitchEntry { name: "HIGHRAM",   active: has(0x0020), address: "$C080-$C08F" },
        SwitchEntry { name: "HIRES",     active: has(0x0040), address: "$C056/$C057" },
        SwitchEntry { name: "PAGE2",     active: has(0x0080), address: "$C054/$C055" },
        SwitchEntry { name: "SLOTC3ROM", active: has(0x0100), address: "$C00A/$C00B" },
        SwitchEntry { name: "INTCXROM",  active: has(0x0200), address: "$C006/$C007" },
        SwitchEntry { name: "WRITERAM",  active: has(0x0400), address: "$C080-$C08F" },
        SwitchEntry { name: "IOUDIS",    active: has(0x0800), address: "$C07E/$C07F" },
        SwitchEntry { name: "GRAPHICS",  active: has(0x4000), address: "$C050/$C051" },
        SwitchEntry { name: "MIXED",     active: has(0x8000), address: "$C052/$C053" },
        SwitchEntry { name: "VID80",     active: has(0x0001_0000), address: "$C00C/$C00D" },
        SwitchEntry { name: "ALTCHAR",   active: has(0x0002_0000), address: "$C00E/$C00F" },
        SwitchEntry { name: "DHIRES",    active: has(0x0004_0000), address: "$C05E/$C05F" },
    ];

    SoftSwitchInfo { items }
}
