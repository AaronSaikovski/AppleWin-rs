//! 6502 / 65C02 disassembler.
//!
//! Reference: `source/Debugger/Debugger_Disassembler.cpp`

/// Instruction addressing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrMode {
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    Indirect,
    IndirectX,  // (zp,X)
    IndirectY,  // (zp),Y
    IndirectZp, // 65C02: (zp)
    IndAbsX,    // 65C02: (abs,X)
    Relative,
}

/// One decoded instruction.
#[derive(Debug, Clone)]
pub struct Instruction {
    pub addr: u16,
    pub opcode: u8,
    pub mnemonic: &'static str,
    pub mode: AddrMode,
    pub operand: u32, // up to 2 bytes
    pub bytes: u8,    // total instruction length
}

/// Mnemonic and addressing mode table entry.
struct OpInfo {
    mnemonic: &'static str,
    mode: AddrMode,
}

macro_rules! op {
    ($m:expr, $mode:ident) => {
        OpInfo {
            mnemonic: $m,
            mode: AddrMode::$mode,
        }
    };
}

/// 6502 / 65C02 opcode table (65C02 superset).
#[rustfmt::skip]
static OPCODES: [OpInfo; 256] = [
    op!("BRK", Implied),    op!("ORA", IndirectX),  op!("???", Implied),    op!("???", Implied),   // 00
    op!("TSB", ZeroPage),   op!("ORA", ZeroPage),   op!("ASL", ZeroPage),   op!("???", Implied),   // 04
    op!("PHP", Implied),    op!("ORA", Immediate),  op!("ASL", Accumulator),op!("???", Implied),   // 08
    op!("TSB", Absolute),   op!("ORA", Absolute),   op!("ASL", Absolute),   op!("???", Implied),   // 0C
    op!("BPL", Relative),   op!("ORA", IndirectY),  op!("ORA", IndirectZp), op!("???", Implied),   // 10
    op!("TRB", ZeroPage),   op!("ORA", ZeroPageX),  op!("ASL", ZeroPageX),  op!("???", Implied),   // 14
    op!("CLC", Implied),    op!("ORA", AbsoluteY),  op!("INC", Accumulator),op!("???", Implied),   // 18
    op!("TRB", Absolute),   op!("ORA", AbsoluteX),  op!("ASL", AbsoluteX),  op!("???", Implied),   // 1C
    op!("JSR", Absolute),   op!("AND", IndirectX),  op!("???", Implied),    op!("???", Implied),   // 20
    op!("BIT", ZeroPage),   op!("AND", ZeroPage),   op!("ROL", ZeroPage),   op!("???", Implied),   // 24
    op!("PLP", Implied),    op!("AND", Immediate),  op!("ROL", Accumulator),op!("???", Implied),   // 28
    op!("BIT", Absolute),   op!("AND", Absolute),   op!("ROL", Absolute),   op!("???", Implied),   // 2C
    op!("BMI", Relative),   op!("AND", IndirectY),  op!("AND", IndirectZp), op!("???", Implied),   // 30
    op!("BIT", ZeroPageX),  op!("AND", ZeroPageX),  op!("ROL", ZeroPageX),  op!("???", Implied),   // 34
    op!("SEC", Implied),    op!("AND", AbsoluteY),  op!("DEC", Accumulator),op!("???", Implied),   // 38
    op!("BIT", AbsoluteX),  op!("AND", AbsoluteX),  op!("ROL", AbsoluteX),  op!("???", Implied),   // 3C
    op!("RTI", Implied),    op!("EOR", IndirectX),  op!("???", Implied),    op!("???", Implied),   // 40
    op!("???", Implied),    op!("EOR", ZeroPage),   op!("LSR", ZeroPage),   op!("???", Implied),   // 44
    op!("PHA", Implied),    op!("EOR", Immediate),  op!("LSR", Accumulator),op!("???", Implied),   // 48
    op!("JMP", Absolute),   op!("EOR", Absolute),   op!("LSR", Absolute),   op!("???", Implied),   // 4C
    op!("BVC", Relative),   op!("EOR", IndirectY),  op!("EOR", IndirectZp), op!("???", Implied),   // 50
    op!("???", Implied),    op!("EOR", ZeroPageX),  op!("LSR", ZeroPageX),  op!("???", Implied),   // 54
    op!("CLI", Implied),    op!("EOR", AbsoluteY),  op!("PHY", Implied),    op!("???", Implied),   // 58
    op!("???", Implied),    op!("EOR", AbsoluteX),  op!("LSR", AbsoluteX),  op!("???", Implied),   // 5C
    op!("RTS", Implied),    op!("ADC", IndirectX),  op!("???", Implied),    op!("???", Implied),   // 60
    op!("STZ", ZeroPage),   op!("ADC", ZeroPage),   op!("ROR", ZeroPage),   op!("???", Implied),   // 64
    op!("PLA", Implied),    op!("ADC", Immediate),  op!("ROR", Accumulator),op!("???", Implied),   // 68
    op!("JMP", Indirect),   op!("ADC", Absolute),   op!("ROR", Absolute),   op!("???", Implied),   // 6C
    op!("BVS", Relative),   op!("ADC", IndirectY),  op!("ADC", IndirectZp), op!("???", Implied),   // 70
    op!("STZ", ZeroPageX),  op!("ADC", ZeroPageX),  op!("ROR", ZeroPageX),  op!("???", Implied),   // 74
    op!("SEI", Implied),    op!("ADC", AbsoluteY),  op!("PLY", Implied),    op!("???", Implied),   // 78
    op!("JMP", IndAbsX),    op!("ADC", AbsoluteX),  op!("ROR", AbsoluteX),  op!("???", Implied),   // 7C
    op!("BRA", Relative),   op!("STA", IndirectX),  op!("???", Implied),    op!("???", Implied),   // 80
    op!("STY", ZeroPage),   op!("STA", ZeroPage),   op!("STX", ZeroPage),   op!("???", Implied),   // 84
    op!("DEY", Implied),    op!("BIT", Immediate),  op!("TXA", Implied),    op!("???", Implied),   // 88
    op!("STY", Absolute),   op!("STA", Absolute),   op!("STX", Absolute),   op!("???", Implied),   // 8C
    op!("BCC", Relative),   op!("STA", IndirectY),  op!("STA", IndirectZp), op!("???", Implied),   // 90
    op!("STY", ZeroPageX),  op!("STA", ZeroPageX),  op!("STX", ZeroPageY),  op!("???", Implied),   // 94
    op!("TYA", Implied),    op!("STA", AbsoluteY),  op!("TXS", Implied),    op!("???", Implied),   // 98
    op!("STZ", Absolute),   op!("STA", AbsoluteX),  op!("STZ", AbsoluteX),  op!("???", Implied),   // 9C
    op!("LDY", Immediate),  op!("LDA", IndirectX),  op!("LDX", Immediate),  op!("???", Implied),   // A0
    op!("LDY", ZeroPage),   op!("LDA", ZeroPage),   op!("LDX", ZeroPage),   op!("???", Implied),   // A4
    op!("TAY", Implied),    op!("LDA", Immediate),  op!("TAX", Implied),    op!("???", Implied),   // A8
    op!("LDY", Absolute),   op!("LDA", Absolute),   op!("LDX", Absolute),   op!("???", Implied),   // AC
    op!("BCS", Relative),   op!("LDA", IndirectY),  op!("LDA", IndirectZp), op!("???", Implied),   // B0
    op!("LDY", ZeroPageX),  op!("LDA", ZeroPageX),  op!("LDX", ZeroPageY),  op!("???", Implied),   // B4
    op!("CLV", Implied),    op!("LDA", AbsoluteY),  op!("TSX", Implied),    op!("???", Implied),   // B8
    op!("LDY", AbsoluteX),  op!("LDA", AbsoluteX),  op!("LDX", AbsoluteY),  op!("???", Implied),   // BC
    op!("CPY", Immediate),  op!("CMP", IndirectX),  op!("???", Implied),    op!("???", Implied),   // C0
    op!("CPY", ZeroPage),   op!("CMP", ZeroPage),   op!("DEC", ZeroPage),   op!("???", Implied),   // C4
    op!("INY", Implied),    op!("CMP", Immediate),  op!("DEX", Implied),    op!("???", Implied),   // C8
    op!("CPY", Absolute),   op!("CMP", Absolute),   op!("DEC", Absolute),   op!("???", Implied),   // CC
    op!("BNE", Relative),   op!("CMP", IndirectY),  op!("CMP", IndirectZp), op!("???", Implied),   // D0
    op!("???", Implied),    op!("CMP", ZeroPageX),  op!("DEC", ZeroPageX),  op!("???", Implied),   // D4
    op!("CLD", Implied),    op!("CMP", AbsoluteY),  op!("PHX", Implied),    op!("???", Implied),   // D8
    op!("???", Implied),    op!("CMP", AbsoluteX),  op!("DEC", AbsoluteX),  op!("???", Implied),   // DC
    op!("CPX", Immediate),  op!("SBC", IndirectX),  op!("???", Implied),    op!("???", Implied),   // E0
    op!("CPX", ZeroPage),   op!("SBC", ZeroPage),   op!("INC", ZeroPage),   op!("???", Implied),   // E4
    op!("INX", Implied),    op!("SBC", Immediate),  op!("NOP", Implied),    op!("???", Implied),   // E8
    op!("CPX", Absolute),   op!("SBC", Absolute),   op!("INC", Absolute),   op!("???", Implied),   // EC
    op!("BEQ", Relative),   op!("SBC", IndirectY),  op!("SBC", IndirectZp), op!("???", Implied),   // F0
    op!("???", Implied),    op!("SBC", ZeroPageX),  op!("INC", ZeroPageX),  op!("???", Implied),   // F4
    op!("SED", Implied),    op!("SBC", AbsoluteY),  op!("PLX", Implied),    op!("???", Implied),   // F8
    op!("???", Implied),    op!("SBC", AbsoluteX),  op!("INC", AbsoluteX),  op!("???", Implied),   // FC
];

/// Number of operand bytes for each addressing mode.
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

/// Disassemble one instruction starting at `addr`.
///
/// `read` is a closure returning a byte from the address space.
pub fn disassemble_one<F>(addr: u16, mut read: F) -> Instruction
where
    F: FnMut(u16) -> u8,
{
    let opcode = read(addr);
    let info = &OPCODES[opcode as usize];
    let op_size = operand_size(info.mode);
    let operand: u32 = match op_size {
        1 => read(addr.wrapping_add(1)) as u32,
        2 => {
            let lo = read(addr.wrapping_add(1)) as u32;
            let hi = read(addr.wrapping_add(2)) as u32;
            (hi << 8) | lo
        }
        _ => 0,
    };
    Instruction {
        addr,
        opcode,
        mnemonic: info.mnemonic,
        mode: info.mode,
        operand,
        bytes: 1 + op_size,
    }
}

/// Format a disassembled instruction as a string.
pub fn format_instruction(instr: &Instruction) -> String {
    let op = instr.operand;
    let target = match instr.mode {
        AddrMode::Relative => {
            let offset = op as i8 as i16;
            let target = (instr.addr as i16).wrapping_add(2).wrapping_add(offset) as u16;
            format!("${:04X}", target)
        }
        AddrMode::Implied => String::new(),
        AddrMode::Accumulator => "A".to_string(),
        AddrMode::Immediate => format!("#${:02X}", op),
        AddrMode::ZeroPage => format!("${:02X}", op),
        AddrMode::ZeroPageX => format!("${:02X},X", op),
        AddrMode::ZeroPageY => format!("${:02X},Y", op),
        AddrMode::Absolute => format!("${:04X}", op),
        AddrMode::AbsoluteX => format!("${:04X},X", op),
        AddrMode::AbsoluteY => format!("${:04X},Y", op),
        AddrMode::Indirect => format!("(${:04X})", op),
        AddrMode::IndirectX => format!("(${:02X},X)", op),
        AddrMode::IndirectY => format!("(${:02X}),Y", op),
        AddrMode::IndirectZp => format!("(${:02X})", op),
        AddrMode::IndAbsX => format!("(${:04X},X)", op),
    };
    if target.is_empty() {
        format!("{:04X}: {:3}", instr.addr, instr.mnemonic)
    } else {
        format!("{:04X}: {:3} {}", instr.addr, instr.mnemonic, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disasm_lda_immediate() {
        let mem = [0xA9u8, 0x42]; // LDA #$42
        let instr = disassemble_one(0x0300, |a| mem[(a - 0x0300) as usize]);
        assert_eq!(instr.mnemonic, "LDA");
        assert_eq!(instr.operand, 0x42);
        assert_eq!(instr.bytes, 2);
        assert_eq!(format_instruction(&instr), "0300: LDA #$42");
    }

    #[test]
    fn disasm_jsr() {
        let mem = [0x20u8, 0x00, 0xE0]; // JSR $E000
        let instr = disassemble_one(0x0800, |a| mem[(a - 0x0800) as usize]);
        assert_eq!(instr.mnemonic, "JSR");
        assert_eq!(instr.operand, 0xE000);
        assert_eq!(instr.bytes, 3);
    }
}
