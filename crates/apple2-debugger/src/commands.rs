//! Debugger command parser.
//!
//! Reference: `source/Debugger/Debugger_Commands.cpp`

use crate::breakpoint::{Breakpoint, BreakpointKind};
use crate::markup::MarkupKind;
use crate::symbols::SymbolTable;

/// A parsed debugger command.
#[derive(Debug, Clone)]
pub enum DebugCommand {
    /// G [addr] — Resume execution, optionally run until address.
    Go(Option<u16>),
    /// T — Single step (trace).
    Trace,
    /// P — Step over.
    StepOver,
    /// RTS — Step out of subroutine.
    StepOut,
    /// U [addr] — Unassemble (set disassembly cursor).
    Unassemble(Option<u16>),
    /// BP addr — Add opcode breakpoint.
    BreakpointAdd(u16),
    /// BPM addr — Add memory read+write breakpoint.
    BreakpointAddMem(u16),
    /// BPMR addr — Add memory read breakpoint.
    BreakpointAddMemRead(u16),
    /// BPMW addr — Add memory write breakpoint.
    BreakpointAddMemWrite(u16),
    /// BPC index — Clear (remove) breakpoint by index.
    BreakpointClear(usize),
    /// BPL — List all breakpoints.
    BreakpointList,
    /// BPSAVE path — Save breakpoints to file.
    BreakpointSave(String),
    /// BPLOAD path — Load breakpoints from file.
    BreakpointLoad(String),
    /// R [reg val] — Show registers or set a register.
    Register(Option<(String, u16)>),
    /// MD addr [len] — Memory dump.
    MemoryDump(u16, u16),
    /// SYM name addr — Define a symbol.
    SymbolAdd(String, u16),
    /// SYM name — Remove a symbol.
    SymbolRemove(String),
    /// W addr — Add byte watch at address.
    WatchAdd(u16),
    /// WW addr — Add word watch at address.
    WatchAddWord(u16),
    /// WR reg — Add register watch.
    WatchAddReg(String),
    /// WC [index] — Clear watch (all if no index).
    WatchClear(Option<usize>),
    /// WL — List watches.
    WatchList,
    /// CYCLES — Show cycle counter. CYCLES RESET resets it.
    Cycles(bool),
    /// A addr instruction — Assemble one instruction.
    Assemble(u16, String),
    /// Z addr len — Mark range as data bytes.
    MarkData(u16, u16, MarkupKind),
    /// X addr — Mark as code (remove data markup).
    MarkCode(u16),
    /// NOP addr [count] — Zap instruction(s) with NOP.
    Nop(u16, u16),
    /// F addr len val — Fill memory.
    Fill(u16, u16, u8),
    /// HELP — Show help.
    Help,
}

/// Parse a command string into a `DebugCommand`.
///
/// Addresses can be hex (with or without `$` prefix) or symbol names.
pub fn parse_command(input: &str, symbols: &SymbolTable) -> Result<DebugCommand, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(String::new()); // empty input, not an error
    }

    let mut parts = input.split_whitespace();
    let cmd = parts.next().unwrap().to_uppercase();

    match cmd.as_str() {
        "G" | "GO" => {
            let addr = parse_optional_addr(&mut parts, symbols)?;
            Ok(DebugCommand::Go(addr))
        }
        "T" | "TRACE" => Ok(DebugCommand::Trace),
        "P" => Ok(DebugCommand::StepOver),
        "RTS" => Ok(DebugCommand::StepOut),
        "U" | "UNASM" | "DISASM" => {
            let addr = parse_optional_addr(&mut parts, symbols)?;
            Ok(DebugCommand::Unassemble(addr))
        }
        "BP" | "BPA" | "BPX" => {
            let addr = parse_required_addr(&mut parts, symbols, "BP requires an address")?;
            Ok(DebugCommand::BreakpointAdd(addr))
        }
        "BPM" => {
            let addr = parse_required_addr(&mut parts, symbols, "BPM requires an address")?;
            Ok(DebugCommand::BreakpointAddMem(addr))
        }
        "BPMR" => {
            let addr = parse_required_addr(&mut parts, symbols, "BPMR requires an address")?;
            Ok(DebugCommand::BreakpointAddMemRead(addr))
        }
        "BPMW" => {
            let addr = parse_required_addr(&mut parts, symbols, "BPMW requires an address")?;
            Ok(DebugCommand::BreakpointAddMemWrite(addr))
        }
        "BPC" => {
            let idx_str = parts.next().ok_or("BPC requires a breakpoint index")?;
            let idx: usize = idx_str.parse().map_err(|_| format!("Invalid index: {idx_str}"))?;
            Ok(DebugCommand::BreakpointClear(idx))
        }
        "BPL" => Ok(DebugCommand::BreakpointList),
        "R" | "REG" => {
            if let Some(reg_name) = parts.next() {
                let val = parse_required_addr(&mut parts, symbols, "R requires a value after register name")?;
                Ok(DebugCommand::Register(Some((reg_name.to_uppercase(), val))))
            } else {
                Ok(DebugCommand::Register(None))
            }
        }
        "MD" | "D" | "DUMP" => {
            let addr = parse_required_addr(&mut parts, symbols, "MD requires an address")?;
            let len = parse_optional_addr(&mut parts, symbols)?.unwrap_or(0x80);
            Ok(DebugCommand::MemoryDump(addr, len))
        }
        "SYM" => {
            let name = parts.next().ok_or("SYM requires a name")?.to_string();
            if let Some(addr_str) = parts.next() {
                let addr = parse_addr_str(addr_str, symbols)?;
                Ok(DebugCommand::SymbolAdd(name, addr))
            } else {
                Ok(DebugCommand::SymbolRemove(name))
            }
        }
        "BPSAVE" => {
            let path = parts.next().ok_or("BPSAVE requires a file path")?.to_string();
            Ok(DebugCommand::BreakpointSave(path))
        }
        "BPLOAD" => {
            let path = parts.next().ok_or("BPLOAD requires a file path")?.to_string();
            Ok(DebugCommand::BreakpointLoad(path))
        }
        "W" | "WA" | "WATCH" => {
            let addr = parse_required_addr(&mut parts, symbols, "W requires an address")?;
            Ok(DebugCommand::WatchAdd(addr))
        }
        "WW" => {
            let addr = parse_required_addr(&mut parts, symbols, "WW requires an address")?;
            Ok(DebugCommand::WatchAddWord(addr))
        }
        "WR" => {
            let reg = parts.next().ok_or("WR requires a register name")?.to_uppercase();
            Ok(DebugCommand::WatchAddReg(reg))
        }
        "WC" => {
            let idx = if let Some(s) = parts.next() {
                Some(s.parse::<usize>().map_err(|_| format!("Invalid index: {s}"))?)
            } else {
                None
            };
            Ok(DebugCommand::WatchClear(idx))
        }
        "WL" => Ok(DebugCommand::WatchList),
        "CYCLES" => {
            let reset = parts.next().map(|s| s.eq_ignore_ascii_case("RESET")).unwrap_or(false);
            Ok(DebugCommand::Cycles(reset))
        }
        "A" | "ASM" | "ASSEMBLE" => {
            let addr = parse_required_addr(&mut parts, symbols, "A requires an address")?;
            let instruction: String = parts.collect::<Vec<_>>().join(" ");
            if instruction.is_empty() {
                return Err("A requires an instruction after the address".into());
            }
            Ok(DebugCommand::Assemble(addr, instruction))
        }
        "Z" | "DB" => {
            let addr = parse_required_addr(&mut parts, symbols, "Z/DB requires an address")?;
            let len = parse_optional_addr(&mut parts, symbols)?.unwrap_or(1);
            Ok(DebugCommand::MarkData(addr, len, MarkupKind::Bytes))
        }
        "DW" => {
            let addr = parse_required_addr(&mut parts, symbols, "DW requires an address")?;
            let len = parse_optional_addr(&mut parts, symbols)?.unwrap_or(2);
            Ok(DebugCommand::MarkData(addr, len, MarkupKind::Words))
        }
        "ASC" => {
            let addr = parse_required_addr(&mut parts, symbols, "ASC requires an address")?;
            let len = parse_optional_addr(&mut parts, symbols)?.unwrap_or(1);
            Ok(DebugCommand::MarkData(addr, len, MarkupKind::Ascii))
        }
        "DA" => {
            let addr = parse_required_addr(&mut parts, symbols, "DA requires an address")?;
            let len = parse_optional_addr(&mut parts, symbols)?.unwrap_or(2);
            Ok(DebugCommand::MarkData(addr, len, MarkupKind::Addresses))
        }
        "X" => {
            let addr = parse_required_addr(&mut parts, symbols, "X requires an address")?;
            Ok(DebugCommand::MarkCode(addr))
        }
        "NOP" => {
            let addr = parse_required_addr(&mut parts, symbols, "NOP requires an address")?;
            let count = parse_optional_addr(&mut parts, symbols)?.unwrap_or(1);
            Ok(DebugCommand::Nop(addr, count))
        }
        "F" | "FILL" => {
            let addr = parse_required_addr(&mut parts, symbols, "F requires an address")?;
            let len = parse_required_addr(&mut parts, symbols, "F requires a length")?;
            let val = parse_required_addr(&mut parts, symbols, "F requires a fill value")? as u8;
            Ok(DebugCommand::Fill(addr, len, val))
        }
        "?" | "HELP" | "H" => Ok(DebugCommand::Help),
        _ => Err(format!("Unknown command: {cmd}")),
    }
}

/// Parse an optional hex address or symbol name from the argument stream.
fn parse_optional_addr<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
    symbols: &SymbolTable,
) -> Result<Option<u16>, String> {
    match parts.next() {
        Some(s) => Ok(Some(parse_addr_str(s, symbols)?)),
        None => Ok(None),
    }
}

/// Parse a required hex address or symbol name.
fn parse_required_addr<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
    symbols: &SymbolTable,
    err_msg: &str,
) -> Result<u16, String> {
    let s = parts.next().ok_or_else(|| err_msg.to_string())?;
    parse_addr_str(s, symbols)
}

/// Parse a single address token: hex (with optional `$` prefix) or symbol name.
fn parse_addr_str(s: &str, symbols: &SymbolTable) -> Result<u16, String> {
    let s = s.trim_start_matches('$');
    // Try hex first
    if let Ok(val) = u16::from_str_radix(s, 16) {
        return Ok(val);
    }
    // Try symbol lookup
    if let Some(addr) = symbols.addr_of(s) {
        return Ok(addr);
    }
    Err(format!("Invalid address or unknown symbol: {s}"))
}

/// Format a memory dump as hex + ASCII lines.
pub fn format_memory_dump<F>(start: u16, len: u16, mut read: F) -> Vec<String>
where
    F: FnMut(u16) -> u8,
{
    let mut lines = Vec::new();
    let mut addr = start;
    let end = start.wrapping_add(len);
    while addr != end && (lines.len() < 256) {
        let row_start = addr;
        let mut hex = String::with_capacity(48);
        let mut ascii = String::with_capacity(16);
        for i in 0..16u16 {
            if addr.wrapping_add(i) == end && i > 0 {
                break;
            }
            let b = read(addr.wrapping_add(i));
            hex.push_str(&format!("{:02X} ", b));
            ascii.push(if (0x20..=0x7E).contains(&b) { b as char } else { '.' });
        }
        let bytes_in_row = ascii.len() as u16;
        // Pad hex if less than 16 bytes
        while hex.len() < 48 {
            hex.push(' ');
        }
        lines.push(format!("{:04X}: {}  {}", row_start, hex.trim_end(), ascii));
        addr = addr.wrapping_add(bytes_in_row);
        if bytes_in_row == 0 {
            break;
        }
    }
    lines
}

/// Format the help text listing all available commands.
pub fn help_text() -> Vec<String> {
    vec![
        "--- Execution ---".into(),
        "G [addr]       - Go (resume), optionally run to address".into(),
        "T              - Trace (single step)".into(),
        "P              - Step over".into(),
        "RTS            - Step out of subroutine".into(),
        "U [addr]       - Unassemble at address".into(),
        "--- Breakpoints ---".into(),
        "BP addr        - Add opcode breakpoint".into(),
        "BPM addr       - Add memory read+write breakpoint".into(),
        "BPMR addr      - Add memory read breakpoint".into(),
        "BPMW addr      - Add memory write breakpoint".into(),
        "BPC index      - Clear breakpoint by index".into(),
        "BPL            - List all breakpoints".into(),
        "BPSAVE path    - Save breakpoints to file".into(),
        "BPLOAD path    - Load breakpoints from file".into(),
        "--- Registers & Memory ---".into(),
        "R [reg val]    - Show/set registers (A, X, Y, SP, PC, P)".into(),
        "MD addr [len]  - Memory dump (default 128 bytes)".into(),
        "F addr len val - Fill memory".into(),
        "NOP addr [n]   - Write NOP(s) at address".into(),
        "--- Watches ---".into(),
        "W addr         - Watch byte at address".into(),
        "WW addr        - Watch word at address".into(),
        "WR reg         - Watch register".into(),
        "WC [index]     - Clear watch (all if no index)".into(),
        "WL             - List watches".into(),
        "--- Symbols ---".into(),
        "SYM name addr  - Define symbol".into(),
        "SYM name       - Remove symbol".into(),
        "--- Assembly ---".into(),
        "A addr instr   - Assemble instruction at address".into(),
        "--- Data Markup ---".into(),
        "Z addr [len]   - Mark as data bytes (DB)".into(),
        "DW addr [len]  - Mark as words".into(),
        "ASC addr [len] - Mark as ASCII".into(),
        "DA addr [len]  - Mark as address table".into(),
        "X addr         - Mark as code (clear markup)".into(),
        "--- Misc ---".into(),
        "CYCLES [RESET] - Show/reset cycle counter".into(),
        "?              - This help".into(),
    ]
}

/// Create a breakpoint from a BPM/BPMR/BPMW command.
pub fn make_mem_breakpoint(kind: BreakpointKind, addr: u16) -> Breakpoint {
    Breakpoint {
        kind,
        address: addr,
        length: 1,
        enabled: true,
        label: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_go_no_addr() {
        let sym = SymbolTable::new();
        match parse_command("G", &sym).unwrap() {
            DebugCommand::Go(None) => {}
            other => panic!("Expected Go(None), got {:?}", other),
        }
    }

    #[test]
    fn parse_bp_hex() {
        let sym = SymbolTable::new();
        match parse_command("BP $FA62", &sym).unwrap() {
            DebugCommand::BreakpointAdd(0xFA62) => {}
            other => panic!("Expected BreakpointAdd(0xFA62), got {:?}", other),
        }
    }

    #[test]
    fn parse_bp_symbol() {
        let mut sym = SymbolTable::new();
        sym.insert("GETLN", 0xFD6A);
        match parse_command("BP GETLN", &sym).unwrap() {
            DebugCommand::BreakpointAdd(0xFD6A) => {}
            other => panic!("Expected BreakpointAdd(0xFD6A), got {:?}", other),
        }
    }

    #[test]
    fn parse_memory_dump() {
        let sym = SymbolTable::new();
        match parse_command("MD 400 40", &sym).unwrap() {
            DebugCommand::MemoryDump(0x0400, 0x0040) => {}
            other => panic!("Expected MemoryDump(0x400, 0x40), got {:?}", other),
        }
    }

    #[test]
    fn format_dump() {
        let lines = format_memory_dump(0x0000, 0x20, |a| a as u8);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("0000:"));
    }
}
