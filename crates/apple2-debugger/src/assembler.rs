//! 6502 / 65C02 mini-assembler.
//!
//! Reference: `source/Debugger/Debugger_Assembler.cpp`
//!
//! Parses a single 6502 instruction and emits the corresponding bytes.
//! Supports all standard addressing modes.

use crate::disasm::AddrMode;

/// Result of assembling one instruction.
#[derive(Debug, Clone)]
pub struct AssembledInstruction {
    /// The encoded bytes (1–3).
    pub bytes: Vec<u8>,
}

/// Assemble a single instruction at the given address.
///
/// `input` should be a mnemonic followed by an operand, e.g. "LDA #$42" or "JMP $E000".
/// `addr` is needed to compute relative branch offsets.
///
/// Returns the assembled bytes or an error message.
pub fn assemble_one(input: &str, addr: u16) -> Result<AssembledInstruction, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Empty input".into());
    }

    let (mnemonic, operand) = split_mnemonic_operand(input);
    let mnemonic = mnemonic.to_uppercase();

    let (mode, value) = parse_operand(operand, addr)?;

    // Look up the opcode for this mnemonic + addressing mode.
    let (opcode, actual_mode) = find_opcode(&mnemonic, mode)
        .ok_or_else(|| format!("Invalid: {} with {:?} mode", mnemonic, mode))?;

    let mut bytes = vec![opcode];
    match operand_size(actual_mode) {
        0 => {}
        1 => bytes.push(value as u8),
        2 => {
            bytes.push(value as u8);
            bytes.push((value >> 8) as u8);
        }
        _ => unreachable!(),
    }

    Ok(AssembledInstruction { bytes })
}

fn split_mnemonic_operand(input: &str) -> (&str, &str) {
    if let Some(pos) = input.find(|c: char| c.is_whitespace()) {
        let (m, o) = input.split_at(pos);
        (m.trim(), o.trim())
    } else {
        (input, "")
    }
}

fn operand_size(mode: AddrMode) -> u8 {
    match mode {
        AddrMode::Implied | AddrMode::Accumulator => 0,
        AddrMode::Immediate
        | AddrMode::ZeroPage
        | AddrMode::ZeroPageX
        | AddrMode::ZeroPageY
        | AddrMode::IndirectX
        | AddrMode::IndirectY
        | AddrMode::IndirectZp
        | AddrMode::Relative => 1,
        AddrMode::Absolute
        | AddrMode::AbsoluteX
        | AddrMode::AbsoluteY
        | AddrMode::Indirect
        | AddrMode::IndAbsX => 2,
    }
}

/// Parse the operand string to determine addressing mode and value.
fn parse_operand(operand: &str, _addr: u16) -> Result<(AddrMode, u16), String> {
    let operand = operand.trim();

    if operand.is_empty() {
        return Ok((AddrMode::Implied, 0));
    }

    if operand.eq_ignore_ascii_case("A") {
        return Ok((AddrMode::Accumulator, 0));
    }

    // #$XX — Immediate
    if let Some(rest) = operand.strip_prefix('#') {
        let val = parse_value(rest)?;
        return Ok((AddrMode::Immediate, val));
    }

    // ($XX,X) — Indirect X
    if operand.starts_with('(') && operand.to_uppercase().ends_with(",X)") {
        let inner = &operand[1..operand.len() - 3];
        let val = parse_value(inner)?;
        return Ok((AddrMode::IndirectX, val));
    }

    // ($XX),Y — Indirect Y
    if operand.starts_with('(') && operand.to_uppercase().ends_with("),Y") {
        let close = operand.find(')').ok_or("Missing )")?;
        let inner = &operand[1..close];
        let val = parse_value(inner)?;
        return Ok((AddrMode::IndirectY, val));
    }

    // ($XXXX,X) — Indirect Absolute X (65C02)
    if operand.starts_with('(') && operand.to_uppercase().ends_with(",X)") {
        let inner = &operand[1..operand.len() - 3];
        let val = parse_value(inner)?;
        if val > 0xFF {
            return Ok((AddrMode::IndAbsX, val));
        }
        return Ok((AddrMode::IndirectX, val));
    }

    // ($XX) or ($XXXX) — Indirect ZP or Indirect Absolute
    if operand.starts_with('(') && operand.ends_with(')') {
        let inner = &operand[1..operand.len() - 1];
        let val = parse_value(inner)?;
        if val <= 0xFF {
            return Ok((AddrMode::IndirectZp, val));
        } else {
            return Ok((AddrMode::Indirect, val));
        }
    }

    // Check for ,X or ,Y suffix
    let upper = operand.to_uppercase();
    if let Some(base) = upper.strip_suffix(",X") {
        let val = parse_value(base.trim())?;
        if val <= 0xFF {
            return Ok((AddrMode::ZeroPageX, val));
        } else {
            return Ok((AddrMode::AbsoluteX, val));
        }
    }
    if let Some(base) = upper.strip_suffix(",Y") {
        let val = parse_value(base.trim())?;
        if val <= 0xFF {
            return Ok((AddrMode::ZeroPageY, val));
        } else {
            return Ok((AddrMode::AbsoluteY, val));
        }
    }

    // Plain address — could be ZeroPage, Absolute, or Relative (for branches)
    let val = parse_value(operand)?;
    // We'll return Absolute or ZeroPage; the caller will try Relative for branches
    if val <= 0xFF {
        // Could be zero page — but might need absolute. Return ZP and let
        // find_opcode fall back to absolute if ZP isn't valid for this mnemonic.
        Ok((AddrMode::ZeroPage, val))
    } else {
        Ok((AddrMode::Absolute, val))
    }
}

/// Parse a numeric value: $XX hex, %XXXXXXXX binary, or decimal.
fn parse_value(s: &str) -> Result<u16, String> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('$') {
        u16::from_str_radix(hex, 16).map_err(|_| format!("Invalid hex: {s}"))
    } else if let Some(bin) = s.strip_prefix('%') {
        u16::from_str_radix(bin, 2).map_err(|_| format!("Invalid binary: {s}"))
    } else {
        // Try hex first (common in 6502 context), then decimal
        u16::from_str_radix(s, 16)
            .or_else(|_| s.parse::<u16>())
            .map_err(|_| format!("Invalid value: {s}"))
    }
}

/// Opcode table entry for the assembler: (mnemonic, addressing_mode, opcode_byte).
struct AsmEntry {
    mnemonic: &'static str,
    mode: AddrMode,
    opcode: u8,
}

macro_rules! asm {
    ($m:expr, $mode:ident, $op:expr) => {
        AsmEntry { mnemonic: $m, mode: AddrMode::$mode, opcode: $op }
    };
}

/// Find the opcode byte for a mnemonic + addressing mode.
///
/// Returns `(opcode, actual_mode)` — the actual mode may differ from the
/// requested one (e.g. Relative when Absolute was requested for a branch).
fn find_opcode(mnemonic: &str, mode: AddrMode) -> Option<(u8, AddrMode)> {
    // First try exact match
    if let Some(entry) = ASM_TABLE.iter().find(|e| e.mnemonic == mnemonic && e.mode == mode) {
        return Some((entry.opcode, entry.mode));
    }
    // For branch instructions: if mode is ZeroPage or Absolute, try Relative
    if matches!(mode, AddrMode::ZeroPage | AddrMode::Absolute)
        && let Some(entry) = ASM_TABLE.iter().find(|e| e.mnemonic == mnemonic && e.mode == AddrMode::Relative) {
        return Some((entry.opcode, entry.mode));
    }
    // If mode is ZeroPage but only Absolute exists, try Absolute
    if mode == AddrMode::ZeroPage
        && let Some(entry) = ASM_TABLE.iter().find(|e| e.mnemonic == mnemonic && e.mode == AddrMode::Absolute) {
        return Some((entry.opcode, entry.mode));
    }
    None
}

/// Complete 6502/65C02 opcode table for the assembler.
#[rustfmt::skip]
static ASM_TABLE: &[AsmEntry] = &[
    // ADC
    asm!("ADC", Immediate,  0x69), asm!("ADC", ZeroPage,   0x65), asm!("ADC", ZeroPageX,  0x75),
    asm!("ADC", Absolute,   0x6D), asm!("ADC", AbsoluteX,  0x7D), asm!("ADC", AbsoluteY,  0x79),
    asm!("ADC", IndirectX,  0x61), asm!("ADC", IndirectY,   0x71), asm!("ADC", IndirectZp,  0x72),
    // AND
    asm!("AND", Immediate,  0x29), asm!("AND", ZeroPage,   0x25), asm!("AND", ZeroPageX,  0x35),
    asm!("AND", Absolute,   0x2D), asm!("AND", AbsoluteX,  0x3D), asm!("AND", AbsoluteY,  0x39),
    asm!("AND", IndirectX,  0x21), asm!("AND", IndirectY,   0x31), asm!("AND", IndirectZp,  0x32),
    // ASL
    asm!("ASL", Accumulator,0x0A), asm!("ASL", ZeroPage,   0x06), asm!("ASL", ZeroPageX,  0x16),
    asm!("ASL", Absolute,   0x0E), asm!("ASL", AbsoluteX,  0x1E),
    // Branches
    asm!("BCC", Relative,   0x90), asm!("BCS", Relative,   0xB0), asm!("BEQ", Relative,   0xF0),
    asm!("BMI", Relative,   0x30), asm!("BNE", Relative,   0xD0), asm!("BPL", Relative,   0x10),
    asm!("BVC", Relative,   0x50), asm!("BVS", Relative,   0x70), asm!("BRA", Relative,   0x80),
    // BIT
    asm!("BIT", ZeroPage,   0x24), asm!("BIT", Absolute,   0x2C), asm!("BIT", ZeroPageX,  0x34),
    asm!("BIT", AbsoluteX,  0x3C), asm!("BIT", Immediate,  0x89),
    // BRK
    asm!("BRK", Implied,    0x00),
    // CLC/CLD/CLI/CLV
    asm!("CLC", Implied,    0x18), asm!("CLD", Implied,    0xD8), asm!("CLI", Implied,    0x58),
    asm!("CLV", Implied,    0xB8),
    // CMP
    asm!("CMP", Immediate,  0xC9), asm!("CMP", ZeroPage,   0xC5), asm!("CMP", ZeroPageX,  0xD5),
    asm!("CMP", Absolute,   0xCD), asm!("CMP", AbsoluteX,  0xDD), asm!("CMP", AbsoluteY,  0xD9),
    asm!("CMP", IndirectX,  0xC1), asm!("CMP", IndirectY,   0xD1), asm!("CMP", IndirectZp,  0xD2),
    // CPX
    asm!("CPX", Immediate,  0xE0), asm!("CPX", ZeroPage,   0xE4), asm!("CPX", Absolute,   0xEC),
    // CPY
    asm!("CPY", Immediate,  0xC0), asm!("CPY", ZeroPage,   0xC4), asm!("CPY", Absolute,   0xCC),
    // DEC
    asm!("DEC", Accumulator,0x3A), asm!("DEC", ZeroPage,   0xC6), asm!("DEC", ZeroPageX,  0xD6),
    asm!("DEC", Absolute,   0xCE), asm!("DEC", AbsoluteX,  0xDE),
    // DEX/DEY
    asm!("DEX", Implied,    0xCA), asm!("DEY", Implied,    0x88),
    // EOR
    asm!("EOR", Immediate,  0x49), asm!("EOR", ZeroPage,   0x45), asm!("EOR", ZeroPageX,  0x55),
    asm!("EOR", Absolute,   0x4D), asm!("EOR", AbsoluteX,  0x5D), asm!("EOR", AbsoluteY,  0x59),
    asm!("EOR", IndirectX,  0x41), asm!("EOR", IndirectY,   0x51), asm!("EOR", IndirectZp,  0x52),
    // INC
    asm!("INC", Accumulator,0x1A), asm!("INC", ZeroPage,   0xE6), asm!("INC", ZeroPageX,  0xF6),
    asm!("INC", Absolute,   0xEE), asm!("INC", AbsoluteX,  0xFE),
    // INX/INY
    asm!("INX", Implied,    0xE8), asm!("INY", Implied,    0xC8),
    // JMP
    asm!("JMP", Absolute,   0x4C), asm!("JMP", Indirect,   0x6C), asm!("JMP", IndAbsX,    0x7C),
    // JSR
    asm!("JSR", Absolute,   0x20),
    // LDA
    asm!("LDA", Immediate,  0xA9), asm!("LDA", ZeroPage,   0xA5), asm!("LDA", ZeroPageX,  0xB5),
    asm!("LDA", Absolute,   0xAD), asm!("LDA", AbsoluteX,  0xBD), asm!("LDA", AbsoluteY,  0xB9),
    asm!("LDA", IndirectX,  0xA1), asm!("LDA", IndirectY,   0xB1), asm!("LDA", IndirectZp,  0xB2),
    // LDX
    asm!("LDX", Immediate,  0xA2), asm!("LDX", ZeroPage,   0xA6), asm!("LDX", ZeroPageY,  0xB6),
    asm!("LDX", Absolute,   0xAE), asm!("LDX", AbsoluteY,  0xBE),
    // LDY
    asm!("LDY", Immediate,  0xA0), asm!("LDY", ZeroPage,   0xA4), asm!("LDY", ZeroPageX,  0xB4),
    asm!("LDY", Absolute,   0xAC), asm!("LDY", AbsoluteX,  0xBC),
    // LSR
    asm!("LSR", Accumulator,0x4A), asm!("LSR", ZeroPage,   0x46), asm!("LSR", ZeroPageX,  0x56),
    asm!("LSR", Absolute,   0x4E), asm!("LSR", AbsoluteX,  0x5E),
    // NOP
    asm!("NOP", Implied,    0xEA),
    // ORA
    asm!("ORA", Immediate,  0x09), asm!("ORA", ZeroPage,   0x05), asm!("ORA", ZeroPageX,  0x15),
    asm!("ORA", Absolute,   0x0D), asm!("ORA", AbsoluteX,  0x1D), asm!("ORA", AbsoluteY,  0x19),
    asm!("ORA", IndirectX,  0x01), asm!("ORA", IndirectY,   0x11), asm!("ORA", IndirectZp,  0x12),
    // PHA/PHP/PHX/PHY / PLA/PLP/PLX/PLY
    asm!("PHA", Implied,    0x48), asm!("PHP", Implied,    0x08),
    asm!("PHX", Implied,    0xDA), asm!("PHY", Implied,    0x5A),
    asm!("PLA", Implied,    0x68), asm!("PLP", Implied,    0x28),
    asm!("PLX", Implied,    0xFA), asm!("PLY", Implied,    0x7A),
    // ROL
    asm!("ROL", Accumulator,0x2A), asm!("ROL", ZeroPage,   0x26), asm!("ROL", ZeroPageX,  0x36),
    asm!("ROL", Absolute,   0x2E), asm!("ROL", AbsoluteX,  0x3E),
    // ROR
    asm!("ROR", Accumulator,0x6A), asm!("ROR", ZeroPage,   0x66), asm!("ROR", ZeroPageX,  0x76),
    asm!("ROR", Absolute,   0x6E), asm!("ROR", AbsoluteX,  0x7E),
    // RTI/RTS
    asm!("RTI", Implied,    0x40), asm!("RTS", Implied,    0x60),
    // SBC
    asm!("SBC", Immediate,  0xE9), asm!("SBC", ZeroPage,   0xE5), asm!("SBC", ZeroPageX,  0xF5),
    asm!("SBC", Absolute,   0xED), asm!("SBC", AbsoluteX,  0xFD), asm!("SBC", AbsoluteY,  0xF9),
    asm!("SBC", IndirectX,  0xE1), asm!("SBC", IndirectY,   0xF1), asm!("SBC", IndirectZp,  0xF2),
    // SEC/SED/SEI
    asm!("SEC", Implied,    0x38), asm!("SED", Implied,    0xF8), asm!("SEI", Implied,    0x78),
    // STA
    asm!("STA", ZeroPage,   0x85), asm!("STA", ZeroPageX,  0x95),
    asm!("STA", Absolute,   0x8D), asm!("STA", AbsoluteX,  0x9D), asm!("STA", AbsoluteY,  0x99),
    asm!("STA", IndirectX,  0x81), asm!("STA", IndirectY,   0x91), asm!("STA", IndirectZp,  0x92),
    // STX
    asm!("STX", ZeroPage,   0x86), asm!("STX", ZeroPageY,  0x96), asm!("STX", Absolute,   0x8E),
    // STY
    asm!("STY", ZeroPage,   0x84), asm!("STY", ZeroPageX,  0x94), asm!("STY", Absolute,   0x8C),
    // STZ (65C02)
    asm!("STZ", ZeroPage,   0x64), asm!("STZ", ZeroPageX,  0x74),
    asm!("STZ", Absolute,   0x9C), asm!("STZ", AbsoluteX,  0x9E),
    // TAX/TAY/TSX/TXA/TXS/TYA
    asm!("TAX", Implied,    0xAA), asm!("TAY", Implied,    0xA8), asm!("TSX", Implied,    0xBA),
    asm!("TXA", Implied,    0x8A), asm!("TXS", Implied,    0x9A), asm!("TYA", Implied,    0x98),
    // TRB/TSB (65C02)
    asm!("TRB", ZeroPage,   0x14), asm!("TRB", Absolute,   0x1C),
    asm!("TSB", ZeroPage,   0x04), asm!("TSB", Absolute,   0x0C),
];

/// Compute the relative branch offset for a branch instruction.
/// `addr` is the address of the branch instruction, `target` is the destination.
/// The branch is relative to addr+2 (after the 2-byte branch instruction).
pub fn relative_offset(addr: u16, target: u16) -> Result<u8, String> {
    let from = addr.wrapping_add(2) as i32;
    let to = target as i32;
    let offset = to - from;
    if !(-128..=127).contains(&offset) {
        return Err(format!("Branch target ${target:04X} out of range from ${addr:04X} (offset {offset})"));
    }
    Ok(offset as i8 as u8)
}

/// Assemble one instruction, handling branches correctly.
///
/// For branch instructions, the operand value is the absolute target address,
/// and this function computes the relative offset.
pub fn assemble_at(input: &str, addr: u16) -> Result<AssembledInstruction, String> {
    let mut result = assemble_one(input, addr)?;

    // Check if this is a branch instruction (1-byte operand, Relative mode).
    // If the assembled bytes are [opcode, abs_lo] and the opcode is a branch,
    // we need to convert the absolute target to a relative offset.
    if result.bytes.len() == 2 {
        let opcode = result.bytes[0];
        let is_branch = BRANCH_OPCODES.contains(&opcode);
        if is_branch {
            // The value was parsed as an address; compute relative offset
            let (_, operand) = split_mnemonic_operand(input.trim());
            let target = parse_value(operand.trim())
                .map_err(|e| format!("Branch target: {e}"))?;
            let offset = relative_offset(addr, target)?;
            result.bytes[1] = offset;
        }
    }

    Ok(result)
}

const BRANCH_OPCODES: &[u8] = &[
    0x10, // BPL
    0x30, // BMI
    0x50, // BVC
    0x70, // BVS
    0x80, // BRA (65C02)
    0x90, // BCC
    0xB0, // BCS
    0xD0, // BNE
    0xF0, // BEQ
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asm_lda_immediate() {
        let result = assemble_one("LDA #$42", 0x0300).unwrap();
        assert_eq!(result.bytes, vec![0xA9, 0x42]);
    }

    #[test]
    fn asm_jsr_absolute() {
        let result = assemble_one("JSR $E000", 0x0300).unwrap();
        assert_eq!(result.bytes, vec![0x20, 0x00, 0xE0]);
    }

    #[test]
    fn asm_sta_indirect_y() {
        let result = assemble_one("STA ($06),Y", 0x0300).unwrap();
        assert_eq!(result.bytes, vec![0x91, 0x06]);
    }

    #[test]
    fn asm_nop() {
        let result = assemble_one("NOP", 0x0300).unwrap();
        assert_eq!(result.bytes, vec![0xEA]);
    }

    #[test]
    fn asm_branch() {
        // BEQ to $0305 from $0300 → offset = $05 - $02 = $03
        let result = assemble_at("BEQ $0305", 0x0300).unwrap();
        assert_eq!(result.bytes, vec![0xF0, 0x03]);
    }

    #[test]
    fn asm_branch_backward() {
        // BNE to $0300 from $0310 → offset = $0300 - $0312 = -18 = $EE
        let result = assemble_at("BNE $0300", 0x0310).unwrap();
        assert_eq!(result.bytes, vec![0xD0, 0xEE]);
    }

    #[test]
    fn asm_lda_zp_x() {
        let result = assemble_one("LDA $10,X", 0x0300).unwrap();
        assert_eq!(result.bytes, vec![0xB5, 0x10]);
    }

    #[test]
    fn asm_stx_zp_y() {
        let result = assemble_one("STX $10,Y", 0x0300).unwrap();
        assert_eq!(result.bytes, vec![0x96, 0x10]);
    }
}
