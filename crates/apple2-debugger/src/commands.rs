//! Debugger command parser and executor.
//!
//! Implements an AppleWin-compatible command set.
//! Reference: `source/Debugger/Debugger_Commands.cpp`

use crate::breakpoint::{Breakpoint, BreakpointKind};
use crate::disasm::{disassemble_one, format_instruction};
use crate::state::DebuggerState;

/// Result of executing a debugger command.
#[derive(Debug)]
pub enum CmdResult {
    /// Print lines to the console output.
    Output(Vec<String>),
    /// Resume execution (Go).
    Go,
    /// Step one instruction.
    Step,
    /// Step over (execute JSR as single step).
    StepOver,
    /// Step out (run until RTS).
    StepOut,
    /// Trace N instructions.
    Trace(u32),
    /// Set PC to address.
    SetPC(u16),
    /// Write a byte to memory.
    MemWrite(u16, u8),
    /// Set a register value: (register_char, value).
    SetReg(char, u16),
    /// Toggle breakpoint active state.
    ToggleBreak,
    /// No operation / empty command.
    Nop,
    /// Error message.
    Error(String),
}

/// Parse a hex value, stripping optional leading '$'.
fn parse_hex(s: &str) -> Option<u16> {
    let s = s.strip_prefix('$').unwrap_or(s);
    u16::from_str_radix(s, 16).ok()
}

/// Parse a decimal or hex value (hex if prefixed with '$' or contains A-F).
fn parse_value(s: &str) -> Option<u16> {
    if s.starts_with('$') {
        parse_hex(s)
    } else if s.chars().any(|c| c.is_ascii_hexdigit() && !c.is_ascii_digit()) {
        u16::from_str_radix(s, 16).ok()
    } else {
        s.parse::<u16>().ok().or_else(|| u16::from_str_radix(s, 16).ok())
    }
}

/// Execute a debugger command string.
///
/// `read_mem` is a closure that reads a byte from the emulator's address space.
/// `pc` is the current program counter.
pub fn execute_command<F>(
    state: &mut DebuggerState,
    input: &str,
    pc: u16,
    regs: CpuRegs,
    mut read_mem: F,
) -> CmdResult
where
    F: FnMut(u16) -> u8,
{
    let input = input.trim();
    if input.is_empty() {
        // Repeat last command
        if state.last_command.is_empty() {
            return CmdResult::Nop;
        }
        let last = state.last_command.clone();
        return execute_command(state, &last, pc, regs, read_mem);
    }

    // Save command for repeat
    state.last_command = input.to_uppercase();
    state.command_history.push(input.to_string());

    let parts: Vec<&str> = input.split_whitespace().collect();
    let cmd = parts[0].to_uppercase();
    let args = &parts[1..];

    match cmd.as_str() {
        // ── Execution ────────────────────────────────────────────────────
        "G" | "GO" => {
            if let Some(addr_str) = args.first() {
                if let Some(addr) = parse_hex(addr_str) {
                    return CmdResult::SetPC(addr);
                }
            }
            CmdResult::Go
        }
        "S" | "STEP" => CmdResult::Step,
        "SO" | "STEPOVER" => CmdResult::StepOver,
        "OUT" | "STEPOUT" => CmdResult::StepOut,
        "T" | "TRACE" => {
            let n = args.first().and_then(|s| parse_value(s)).unwrap_or(1) as u32;
            CmdResult::Trace(n)
        }

        // ── Registers ────────────────────────────────────────────────────
        "R" | "REG" | "REGISTERS" => {
            if let Some(reg_str) = args.first() {
                let reg = reg_str.to_uppercase();
                if let Some(val_str) = args.get(1) {
                    if let Some(val) = parse_hex(val_str) {
                        let ch = reg.chars().next().unwrap_or('?');
                        return CmdResult::SetReg(ch, val);
                    }
                }
                return CmdResult::Error(format!("Usage: R <reg> <value>  (e.g. R A 42)"));
            }
            let lines = format_registers(regs);
            CmdResult::Output(lines)
        }

        // ── Disassembly ──────────────────────────────────────────────────
        "U" | "UNASM" | "DISASM" | "L" | "LIST" => {
            let start = args.first().and_then(|s| parse_hex(s)).unwrap_or(pc);
            let count = args.get(1).and_then(|s| parse_value(s)).unwrap_or(16) as usize;
            let mut lines = Vec::new();
            let mut addr = start;
            for _ in 0..count {
                let instr = disassemble_one(addr, &mut read_mem);
                let marker = if addr == pc { ">" } else { " " };
                let sym = state.symbols.name_at(addr)
                    .map(|s| format!(" ; {s}"))
                    .unwrap_or_default();
                lines.push(format!("{marker}{}{sym}", format_instruction(&instr)));
                addr = addr.wrapping_add(instr.bytes as u16);
            }
            state.cursor = start;
            CmdResult::Output(lines)
        }

        // ── Memory dump ──────────────────────────────────────────────────
        "M" | "MEM" | "MD" | "MEMORY" => {
            let start = args.first().and_then(|s| parse_hex(s))
                .unwrap_or(state.mem_dump_addr);
            let count = args.get(1).and_then(|s| parse_value(s)).unwrap_or(128) as usize;
            let lines = format_mem_dump(start, count, &mut read_mem);
            state.mem_dump_addr = start.wrapping_add(count as u16);
            CmdResult::Output(lines)
        }

        // ── Memory edit ──────────────────────────────────────────────────
        "ME" | "MEDIT" | "E" | "ENTER" => {
            if args.len() < 2 {
                return CmdResult::Error("Usage: ME <addr> <byte> [byte...]".into());
            }
            if let Some(addr) = parse_hex(args[0]) {
                for (i, val_str) in args[1..].iter().enumerate() {
                    if let Some(val) = parse_hex(val_str) {
                        let a = addr.wrapping_add(i as u16);
                        return CmdResult::MemWrite(a, val as u8);
                    }
                }
            }
            CmdResult::Error("Invalid address or value".into())
        }

        // ── Breakpoints ──────────────────────────────────────────────────
        "BP" | "BRK" => {
            if let Some(addr_str) = args.first() {
                if let Some(addr) = parse_hex(addr_str) {
                    let bp = Breakpoint::at(addr);
                    let idx = state.breakpoints.add(bp);
                    return CmdResult::Output(vec![format!("Breakpoint #{idx} set at ${addr:04X}")]);
                }
            }
            // List breakpoints
            let lines = list_breakpoints(state);
            CmdResult::Output(lines)
        }
        "BPM" => {
            // Break on memory access
            if let Some(addr_str) = args.first() {
                if let Some(addr) = parse_hex(addr_str) {
                    let kind = if args.get(1).is_some_and(|s| s.eq_ignore_ascii_case("W")) {
                        BreakpointKind::MemWrite
                    } else {
                        BreakpointKind::MemRead
                    };
                    let bp = Breakpoint { kind, address: addr, length: 1, enabled: true, label: None };
                    let idx = state.breakpoints.add(bp);
                    return CmdResult::Output(vec![format!("Memory breakpoint #{idx} at ${addr:04X}")]);
                }
            }
            CmdResult::Error("Usage: BPM <addr> [R|W]".into())
        }
        "BPC" => {
            // Clear breakpoint
            if let Some(idx_str) = args.first() {
                if let Some(idx) = parse_value(idx_str) {
                    state.breakpoints.remove(idx as usize);
                    return CmdResult::Output(vec![format!("Breakpoint #{idx} cleared")]);
                }
                if *idx_str == "*" {
                    state.breakpoints.clear_all();
                    return CmdResult::Output(vec!["All breakpoints cleared".into()]);
                }
            }
            CmdResult::Error("Usage: BPC <index> | BPC *".into())
        }
        "BPD" => {
            // Disable breakpoint
            if let Some(idx_str) = args.first() {
                if let Some(idx) = parse_value(idx_str) {
                    state.breakpoints.set_enabled(idx as usize, false);
                    return CmdResult::Output(vec![format!("Breakpoint #{idx} disabled")]);
                }
            }
            CmdResult::Error("Usage: BPD <index>".into())
        }
        "BPE" => {
            // Enable breakpoint
            if let Some(idx_str) = args.first() {
                if let Some(idx) = parse_value(idx_str) {
                    state.breakpoints.set_enabled(idx as usize, true);
                    return CmdResult::Output(vec![format!("Breakpoint #{idx} enabled")]);
                }
            }
            CmdResult::Error("Usage: BPE <index>".into())
        }
        "BPL" => {
            let lines = list_breakpoints(state);
            CmdResult::Output(lines)
        }

        // ── Watch points ─────────────────────────────────────────────────
        "W" | "WATCH" => {
            if let Some(addr_str) = args.first() {
                if let Some(addr) = parse_hex(addr_str) {
                    let len = args.get(1).and_then(|s| parse_value(s)).unwrap_or(1);
                    state.watches.add(addr, len);
                    return CmdResult::Output(vec![format!("Watch added at ${addr:04X} len {len}")]);
                }
            }
            // List watches
            let lines = format_watches(state, &mut read_mem);
            CmdResult::Output(lines)
        }
        "WC" => {
            if let Some(idx_str) = args.first() {
                if *idx_str == "*" {
                    state.watches.clear();
                    return CmdResult::Output(vec!["All watches cleared".into()]);
                }
                if let Some(idx) = parse_value(idx_str) {
                    state.watches.remove(idx as usize);
                    return CmdResult::Output(vec![format!("Watch #{idx} cleared")]);
                }
            }
            CmdResult::Error("Usage: WC <index> | WC *".into())
        }

        // ── Symbols ──────────────────────────────────────────────────────
        "SYM" | "SYMBOL" => {
            if args.len() >= 2 {
                if let Some(addr) = parse_hex(args[1]) {
                    state.symbols.insert(args[0], addr);
                    return CmdResult::Output(vec![format!("{} = ${addr:04X}", args[0])]);
                }
            }
            if let Some(name) = args.first() {
                if let Some(addr) = state.symbols.addr_of(name) {
                    return CmdResult::Output(vec![format!("{name} = ${addr:04X}")]);
                }
                return CmdResult::Error(format!("Symbol '{name}' not found"));
            }
            CmdResult::Output(vec!["Usage: SYM <name> <addr> | SYM <name>".into()])
        }

        // ── Search memory ────────────────────────────────────────────────
        "F" | "FIND" => {
            if args.len() < 2 {
                return CmdResult::Error("Usage: F <start> <byte> [byte...]".into());
            }
            if let Some(start) = parse_hex(args[0]) {
                let pattern: Vec<u8> = args[1..].iter()
                    .filter_map(|s| parse_hex(s).map(|v| v as u8))
                    .collect();
                if pattern.is_empty() {
                    return CmdResult::Error("No valid bytes to search for".into());
                }
                let results = search_memory(start, &pattern, &mut read_mem);
                if results.is_empty() {
                    return CmdResult::Output(vec!["Not found".into()]);
                }
                let lines: Vec<String> = results.iter()
                    .take(16)
                    .map(|&a| format!("  Found at ${a:04X}"))
                    .collect();
                return CmdResult::Output(lines);
            }
            CmdResult::Error("Invalid start address".into())
        }

        // ── Stack display ────────────────────────────────────────────────
        "STACK" | "K" => {
            let lines = format_stack(regs.sp, &mut read_mem);
            CmdResult::Output(lines)
        }

        // ── Zero page ────────────────────────────────────────────────────
        "ZP" => {
            let start = args.first().and_then(|s| parse_hex(s)).unwrap_or(0);
            let lines = format_mem_dump(start, 256, &mut read_mem);
            CmdResult::Output(lines)
        }

        // ── Soft switches info ───────────────────────────────────────────
        "SS" | "SOFTSWITCH" => {
            CmdResult::Output(vec!["Use the Soft Switches panel to view current state".into()])
        }

        // ── Fill memory ──────────────────────────────────────────────────
        "FILL" => {
            if args.len() < 3 {
                return CmdResult::Error("Usage: FILL <start> <end> <byte>".into());
            }
            // Fill is handled by the GUI layer writing bytes in a loop
            CmdResult::Output(vec!["FILL not yet implemented in console".into()])
        }

        // ── Help ─────────────────────────────────────────────────────────
        "H" | "HELP" | "?" => {
            CmdResult::Output(help_text())
        }

        _ => CmdResult::Error(format!("Unknown command: {cmd}")),
    }
}

/// CPU register snapshot for the command parser.
#[derive(Debug, Clone, Copy)]
pub struct CpuRegs {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub flags: u8,
    pub cycles: u64,
}

fn format_registers(r: CpuRegs) -> Vec<String> {
    let n = if r.flags & 0x80 != 0 { 'N' } else { '.' };
    let v = if r.flags & 0x40 != 0 { 'V' } else { '.' };
    let b = if r.flags & 0x10 != 0 { 'B' } else { '.' };
    let d = if r.flags & 0x08 != 0 { 'D' } else { '.' };
    let i = if r.flags & 0x04 != 0 { 'I' } else { '.' };
    let z = if r.flags & 0x02 != 0 { 'Z' } else { '.' };
    let c = if r.flags & 0x01 != 0 { 'C' } else { '.' };
    vec![
        format!("  A={:02X}  X={:02X}  Y={:02X}  SP={:02X}  PC={:04X}", r.a, r.x, r.y, r.sp, r.pc),
        format!("  P={:02X}  [{n}{v}.{b}{d}{i}{z}{c}]  Cycles={}", r.flags, r.cycles),
    ]
}

/// Format a hex memory dump.
pub fn format_mem_dump<F>(start: u16, count: usize, mut read: F) -> Vec<String>
where
    F: FnMut(u16) -> u8,
{
    let mut lines = Vec::new();
    let mut addr = start;
    let rows = (count + 15) / 16;
    for _ in 0..rows {
        let mut hex = String::new();
        let mut ascii = String::new();
        for i in 0..16u16 {
            let a = addr.wrapping_add(i);
            let b = read(a);
            if i == 8 { hex.push(' '); }
            hex.push_str(&format!("{:02X} ", b));
            ascii.push(if (0x20..=0x7E).contains(&b) { b as char } else { '.' });
        }
        lines.push(format!("{:04X}: {hex} |{ascii}|", addr));
        addr = addr.wrapping_add(16);
    }
    lines
}

/// Format the stack contents.
pub fn format_stack<F>(sp: u8, mut read: F) -> Vec<String>
where
    F: FnMut(u16) -> u8,
{
    let mut lines = vec![format!("  Stack (SP=${:02X}):", sp)];
    let top = sp.wrapping_add(1);
    if top == 0 && sp == 0xFF {
        lines.push("  (empty)".into());
        return lines;
    }
    // Show up to 16 bytes from SP+1 to $01FF
    let mut addr = 0x0100u16 | top as u16;
    let limit = 0x01FFu16;
    let mut count = 0;
    while addr <= limit && count < 16 {
        let b = read(addr);
        lines.push(format!("  ${:04X}: {:02X}", addr, b));
        addr += 1;
        count += 1;
    }
    lines
}

fn list_breakpoints(state: &DebuggerState) -> Vec<String> {
    if state.breakpoints.breakpoints.is_empty() {
        return vec!["  No breakpoints set".into()];
    }
    let mut lines = vec!["  # Addr   Kind     Enabled".into()];
    for (i, bp) in state.breakpoints.breakpoints.iter().enumerate() {
        let kind = match bp.kind {
            BreakpointKind::Opcode    => "PC      ",
            BreakpointKind::MemRead   => "MemRead ",
            BreakpointKind::MemWrite  => "MemWrite",
            BreakpointKind::Register  => "Register",
            BreakpointKind::Interrupt => "IRQ     ",
            BreakpointKind::IoRead    => "IoRead  ",
            BreakpointKind::IoWrite   => "IoWrite ",
            _ => "Other   ",
        };
        let en = if bp.enabled { "Yes" } else { "No " };
        let lbl = bp.label.as_deref().unwrap_or("");
        lines.push(format!("  {i} ${:04X} {kind} {en}  {lbl}", bp.address));
    }
    lines
}

fn format_watches<F>(state: &DebuggerState, mut read: F) -> Vec<String>
where
    F: FnMut(u16) -> u8,
{
    if state.watches.items.is_empty() {
        return vec!["  No watches set".into()];
    }
    let mut lines = vec!["  # Addr   Value".into()];
    for (i, w) in state.watches.items.iter().enumerate() {
        let mut vals = String::new();
        for j in 0..w.length {
            let b = read(w.address.wrapping_add(j));
            vals.push_str(&format!("{:02X} ", b));
        }
        lines.push(format!("  {i} ${:04X}: {vals}", w.address));
    }
    lines
}

/// Search memory for a byte pattern starting at `start`.
fn search_memory<F>(start: u16, pattern: &[u8], mut read: F) -> Vec<u16>
where
    F: FnMut(u16) -> u8,
{
    let mut results = Vec::new();
    let len = pattern.len() as u16;
    let mut addr = start;
    let end = 0xFFFFu32 - len as u32 + 1;
    loop {
        let mut matched = true;
        for (i, &p) in pattern.iter().enumerate() {
            if read(addr.wrapping_add(i as u16)) != p {
                matched = false;
                break;
            }
        }
        if matched {
            results.push(addr);
            if results.len() >= 64 { break; }
        }
        if (addr as u32) >= end { break; }
        addr = addr.wrapping_add(1);
        if addr == 0 { break; } // wrapped around
    }
    results
}

fn help_text() -> Vec<String> {
    vec![
        "Debugger Commands:".into(),
        "  G [addr]          Go / Resume (optionally set PC first)".into(),
        "  S                 Step one instruction".into(),
        "  SO                Step over (skip JSR)".into(),
        "  OUT               Step out (run until RTS)".into(),
        "  T [n]             Trace n instructions (default 1)".into(),
        "  R                 Show registers".into(),
        "  R <reg> <val>     Set register (A, X, Y, SP, PC)".into(),
        "  U [addr] [n]      Disassemble n instructions at addr".into(),
        "  M [addr] [n]      Memory dump (hex + ASCII)".into(),
        "  ME <addr> <byte>  Edit memory".into(),
        "  BP [addr]         Set/list breakpoints".into(),
        "  BPM <addr> [R|W]  Memory access breakpoint".into(),
        "  BPC <#|*>         Clear breakpoint(s)".into(),
        "  BPD <#>           Disable breakpoint".into(),
        "  BPE <#>           Enable breakpoint".into(),
        "  BPL               List breakpoints".into(),
        "  W [addr] [len]    Add/list watches".into(),
        "  WC <#|*>          Clear watch(es)".into(),
        "  F <start> <bytes> Find bytes in memory".into(),
        "  SYM <name> <addr> Define symbol".into(),
        "  STACK             Show stack contents".into(),
        "  ZP [addr]         Show zero page".into(),
        "  H / HELP          This help".into(),
    ]
}
