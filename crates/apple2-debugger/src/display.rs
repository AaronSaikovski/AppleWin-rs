//! Debugger display renderer.
//!
//! Renders the debugger directly into a 560×384 ABGR8888 framebuffer,
//! matching the original AppleWin debugger which replaces the Apple II
//! screen when active.
//!
//! Layout (matching C++ `source/Debugger/Debugger_Display.cpp`):
//!   - Font: 7×8 pixel monospace glyphs → 80 columns × 48 rows
//!   - Left (cols 0–49):  Disassembly listing
//!   - Right (cols 51–79): Registers, Stack, Breakpoints, Watches, Switches
//!   - Bottom rows:        Data window (memory hex dump)
//!   - Last 3 rows:        Console output + command input

use crate::breakpoint::BreakpointKind;
use crate::disasm::{disassemble_one, format_instruction};
use crate::softswitch::decode_soft_switches;
use crate::state::DebuggerState;

// ── Dimensions ──────────────────────────────────────────────────────────────

pub const FB_W: usize = 560;
pub const FB_H: usize = 384;
const CHAR_W: usize = 7;
const CHAR_H: usize = 8;
const COLS: usize = FB_W / CHAR_W; // 80
const ROWS: usize = FB_H / CHAR_H; // 48

/// Column where the right info panel starts.
const INFO_COL: usize = 51;
/// Row where the data window starts.
const DATA_ROW: usize = 36;
/// Row where the console starts.
const CONSOLE_ROW: usize = 44;
/// Number of disassembly lines.
const DISASM_LINES: usize = 34;

// ── AppleWin debugger color palette ─────────────────────────────────────────
// Format: 0xFFBBGGRR (ABGR8888, little-endian → in-memory [R,G,B,A])

const BLACK:     u32 = 0xFF000000;
const RED:       u32 = 0xFF2020FF;
const GREEN:     u32 = 0xFF00FF00;
const YELLOW:    u32 = 0xFF00FFFF;
#[allow(dead_code)]
const BLUE:      u32 = 0xFFFF4040;
#[allow(dead_code)]
const MAGENTA:   u32 = 0xFFFF00FF;
const CYAN:      u32 = 0xFFFFFF00;
const WHITE:     u32 = 0xFFFFFFFF;
const ORANGE:    u32 = 0xFF0080FF;
const GREY:      u32 = 0xFF808080;
const LT_BLUE:   u32 = 0xFFFFC050;

const BG_DEFAULT:     u32 = BLACK;
const BG_PC_LINE:     u32 = 0xFF008080; // dark yellow/olive background for PC
const BG_BP_LINE:     u32 = 0xFF000060; // dark red background for breakpoints
const BG_CURSOR_LINE: u32 = 0xFF402020; // dark blue background for cursor

// ── Built-in 7×8 debug font ────────────────────────────────────────────────
// Procedurally generated minimal bitmap font for ASCII 0x20–0x7E.

/// Get the 7-byte bitmap for a character (one byte per row, MSB=leftmost).
fn glyph(ch: u8) -> [u8; 7] {
    // Minimal 5×7 font data for printable ASCII, packed into 7 rows.
    // Each byte: bits 6..0 represent pixels left-to-right (bit6=leftmost).
    // Characters outside 0x20..0x7E render as a filled box.
    #[rustfmt::skip]
    static FONT: [[u8; 7]; 95] = [
        [0x00,0x00,0x00,0x00,0x00,0x00,0x00], // ' '
        [0x08,0x08,0x08,0x08,0x08,0x00,0x08], // '!'
        [0x14,0x14,0x00,0x00,0x00,0x00,0x00], // '"'
        [0x14,0x14,0x3E,0x14,0x3E,0x14,0x14], // '#'
        [0x08,0x1E,0x28,0x1C,0x0A,0x3C,0x08], // '$'
        [0x30,0x32,0x04,0x08,0x10,0x26,0x06], // '%'
        [0x10,0x28,0x28,0x10,0x2A,0x24,0x1A], // '&'
        [0x08,0x08,0x00,0x00,0x00,0x00,0x00], // '\''
        [0x04,0x08,0x10,0x10,0x10,0x08,0x04], // '('
        [0x10,0x08,0x04,0x04,0x04,0x08,0x10], // ')'
        [0x00,0x08,0x2A,0x1C,0x2A,0x08,0x00], // '*'
        [0x00,0x08,0x08,0x3E,0x08,0x08,0x00], // '+'
        [0x00,0x00,0x00,0x00,0x00,0x08,0x10], // ','
        [0x00,0x00,0x00,0x3E,0x00,0x00,0x00], // '-'
        [0x00,0x00,0x00,0x00,0x00,0x00,0x08], // '.'
        [0x00,0x02,0x04,0x08,0x10,0x20,0x00], // '/'
        [0x1C,0x22,0x26,0x2A,0x32,0x22,0x1C], // '0'
        [0x08,0x18,0x08,0x08,0x08,0x08,0x1C], // '1'
        [0x1C,0x22,0x02,0x0C,0x10,0x20,0x3E], // '2'
        [0x1C,0x22,0x02,0x0C,0x02,0x22,0x1C], // '3'
        [0x04,0x0C,0x14,0x24,0x3E,0x04,0x04], // '4'
        [0x3E,0x20,0x3C,0x02,0x02,0x22,0x1C], // '5'
        [0x0C,0x10,0x20,0x3C,0x22,0x22,0x1C], // '6'
        [0x3E,0x02,0x04,0x08,0x10,0x10,0x10], // '7'
        [0x1C,0x22,0x22,0x1C,0x22,0x22,0x1C], // '8'
        [0x1C,0x22,0x22,0x1E,0x02,0x04,0x18], // '9'
        [0x00,0x00,0x08,0x00,0x00,0x08,0x00], // ':'
        [0x00,0x00,0x08,0x00,0x00,0x08,0x10], // ';'
        [0x04,0x08,0x10,0x20,0x10,0x08,0x04], // '<'
        [0x00,0x00,0x3E,0x00,0x3E,0x00,0x00], // '='
        [0x10,0x08,0x04,0x02,0x04,0x08,0x10], // '>'
        [0x1C,0x22,0x02,0x04,0x08,0x00,0x08], // '?'
        [0x1C,0x22,0x2E,0x2A,0x2E,0x20,0x1C], // '@'
        [0x1C,0x22,0x22,0x3E,0x22,0x22,0x22], // 'A'
        [0x3C,0x22,0x22,0x3C,0x22,0x22,0x3C], // 'B'
        [0x1C,0x22,0x20,0x20,0x20,0x22,0x1C], // 'C'
        [0x3C,0x22,0x22,0x22,0x22,0x22,0x3C], // 'D'
        [0x3E,0x20,0x20,0x3C,0x20,0x20,0x3E], // 'E'
        [0x3E,0x20,0x20,0x3C,0x20,0x20,0x20], // 'F'
        [0x1C,0x22,0x20,0x2E,0x22,0x22,0x1E], // 'G'
        [0x22,0x22,0x22,0x3E,0x22,0x22,0x22], // 'H'
        [0x1C,0x08,0x08,0x08,0x08,0x08,0x1C], // 'I'
        [0x0E,0x04,0x04,0x04,0x04,0x24,0x18], // 'J'
        [0x22,0x24,0x28,0x30,0x28,0x24,0x22], // 'K'
        [0x20,0x20,0x20,0x20,0x20,0x20,0x3E], // 'L'
        [0x22,0x36,0x2A,0x22,0x22,0x22,0x22], // 'M'
        [0x22,0x32,0x2A,0x26,0x22,0x22,0x22], // 'N'
        [0x1C,0x22,0x22,0x22,0x22,0x22,0x1C], // 'O'
        [0x3C,0x22,0x22,0x3C,0x20,0x20,0x20], // 'P'
        [0x1C,0x22,0x22,0x22,0x2A,0x24,0x1A], // 'Q'
        [0x3C,0x22,0x22,0x3C,0x28,0x24,0x22], // 'R'
        [0x1C,0x22,0x20,0x1C,0x02,0x22,0x1C], // 'S'
        [0x3E,0x08,0x08,0x08,0x08,0x08,0x08], // 'T'
        [0x22,0x22,0x22,0x22,0x22,0x22,0x1C], // 'U'
        [0x22,0x22,0x22,0x22,0x14,0x14,0x08], // 'V'
        [0x22,0x22,0x22,0x2A,0x2A,0x36,0x22], // 'W'
        [0x22,0x22,0x14,0x08,0x14,0x22,0x22], // 'X'
        [0x22,0x22,0x14,0x08,0x08,0x08,0x08], // 'Y'
        [0x3E,0x02,0x04,0x08,0x10,0x20,0x3E], // 'Z'
        [0x1C,0x10,0x10,0x10,0x10,0x10,0x1C], // '['
        [0x00,0x20,0x10,0x08,0x04,0x02,0x00], // '\\'
        [0x1C,0x04,0x04,0x04,0x04,0x04,0x1C], // ']'
        [0x08,0x14,0x22,0x00,0x00,0x00,0x00], // '^'
        [0x00,0x00,0x00,0x00,0x00,0x00,0x3E], // '_'
        [0x10,0x08,0x00,0x00,0x00,0x00,0x00], // '`'
        [0x00,0x00,0x1C,0x02,0x1E,0x22,0x1E], // 'a'
        [0x20,0x20,0x3C,0x22,0x22,0x22,0x3C], // 'b'
        [0x00,0x00,0x1C,0x20,0x20,0x20,0x1C], // 'c'
        [0x02,0x02,0x1E,0x22,0x22,0x22,0x1E], // 'd'
        [0x00,0x00,0x1C,0x22,0x3E,0x20,0x1C], // 'e'
        [0x0C,0x10,0x10,0x3C,0x10,0x10,0x10], // 'f'
        [0x00,0x00,0x1E,0x22,0x1E,0x02,0x1C], // 'g'
        [0x20,0x20,0x3C,0x22,0x22,0x22,0x22], // 'h'
        [0x08,0x00,0x18,0x08,0x08,0x08,0x1C], // 'i'
        [0x04,0x00,0x0C,0x04,0x04,0x24,0x18], // 'j'
        [0x20,0x20,0x22,0x24,0x38,0x24,0x22], // 'k'
        [0x18,0x08,0x08,0x08,0x08,0x08,0x1C], // 'l'
        [0x00,0x00,0x36,0x2A,0x2A,0x22,0x22], // 'm'
        [0x00,0x00,0x3C,0x22,0x22,0x22,0x22], // 'n'
        [0x00,0x00,0x1C,0x22,0x22,0x22,0x1C], // 'o'
        [0x00,0x00,0x3C,0x22,0x3C,0x20,0x20], // 'p'
        [0x00,0x00,0x1E,0x22,0x1E,0x02,0x02], // 'q'
        [0x00,0x00,0x2C,0x30,0x20,0x20,0x20], // 'r'
        [0x00,0x00,0x1E,0x20,0x1C,0x02,0x3C], // 's'
        [0x10,0x10,0x3C,0x10,0x10,0x10,0x0C], // 't'
        [0x00,0x00,0x22,0x22,0x22,0x22,0x1E], // 'u'
        [0x00,0x00,0x22,0x22,0x22,0x14,0x08], // 'v'
        [0x00,0x00,0x22,0x22,0x2A,0x2A,0x14], // 'w'
        [0x00,0x00,0x22,0x14,0x08,0x14,0x22], // 'x'
        [0x00,0x00,0x22,0x22,0x1E,0x02,0x1C], // 'y'
        [0x00,0x00,0x3E,0x04,0x08,0x10,0x3E], // 'z'
        [0x04,0x08,0x08,0x10,0x08,0x08,0x04], // '{'
        [0x08,0x08,0x08,0x08,0x08,0x08,0x08], // '|'
        [0x10,0x08,0x08,0x04,0x08,0x08,0x10], // '}'
        [0x00,0x10,0x2A,0x04,0x00,0x00,0x00], // '~'
    ];

    if (0x20..=0x7E).contains(&ch) {
        FONT[(ch - 0x20) as usize]
    } else {
        [0x3E, 0x3E, 0x3E, 0x3E, 0x3E, 0x3E, 0x3E] // filled box
    }
}

// ── Drawing primitives ──────────────────────────────────────────────────────

/// Draw a single character at character cell (col, row).
fn draw_char(fb: &mut [u32], col: usize, row: usize, ch: u8, fg: u32, bg: u32) {
    let px = col * CHAR_W;
    let py = row * CHAR_H;
    let bitmap = glyph(ch);
    for (dy, &row_bits) in bitmap.iter().enumerate().take(CHAR_H) {
        for dx in 0..CHAR_W {
            let pixel = if dy < 7 && dx < 7 {
                if row_bits & (0x20 >> dx) != 0 { fg } else { bg }
            } else {
                bg // right column and bottom row are spacing
            };
            let x = px + dx;
            let y = py + dy;
            if x < FB_W && y < FB_H {
                fb[y * FB_W + x] = pixel;
            }
        }
    }
}

/// Draw a string at (col, row). Returns the column after the last character.
fn draw_str(fb: &mut [u32], col: usize, row: usize, s: &str, fg: u32, bg: u32) -> usize {
    let mut c = col;
    for &b in s.as_bytes() {
        if c >= COLS { break; }
        draw_char(fb, c, row, b, fg, bg);
        c += 1;
    }
    c
}

/// Fill a row range with background color.
fn fill_row(fb: &mut [u32], row: usize, col_start: usize, col_end: usize, bg: u32) {
    for col in col_start..col_end.min(COLS) {
        let px = col * CHAR_W;
        let py = row * CHAR_H;
        for dy in 0..CHAR_H {
            for dx in 0..CHAR_W {
                let x = px + dx;
                let y = py + dy;
                if x < FB_W && y < FB_H {
                    fb[y * FB_W + x] = bg;
                }
            }
        }
    }
}

/// Fill entire framebuffer with a color.
fn fill_all(fb: &mut [u32], color: u32) {
    fb[..FB_W * FB_H].fill(color);
}

/// Draw a horizontal separator line (row of dashes).
fn draw_separator(fb: &mut [u32], row: usize, col_start: usize, col_end: usize) {
    for col in col_start..col_end.min(COLS) {
        draw_char(fb, col, row, b'-', GREY, BG_DEFAULT);
    }
}

// ── CPU register snapshot (passed in from the emulator) ─────────────────────

/// CPU state snapshot for the debugger display.
pub struct CpuSnapshot {
    pub pc: u16,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub flags: u8,
    pub cycles: u64,
}

// ── Main render function ────────────────────────────────────────────────────

/// Render the full debugger display into the framebuffer.
///
/// `fb` must be at least FB_W * FB_H u32 elements.
/// `read_mem` reads a byte from the emulator address space.
/// `cmd_input` is the current command-line input text.
pub fn render<F>(
    fb: &mut [u32],
    state: &DebuggerState,
    cpu: &CpuSnapshot,
    mode_bits: u32,
    cmd_input: &str,
    mut read_mem: F,
)
where
    F: FnMut(u16) -> u8,
{
    fill_all(fb, BG_DEFAULT);

    // ════════════════════════════════════════════════════════════════════════
    // LEFT: Disassembly listing (cols 0–49, rows 0–DISASM_LINES)
    // ════════════════════════════════════════════════════════════════════════
    {
        // Title
        draw_str(fb, 0, 0, " Disassembly", GREEN, BG_DEFAULT);

        let start_addr = if let Some(goto) = state.goto_addr {
            goto
        } else {
            cpu.pc
        };

        let mut addr = start_addr;
        for i in 0..DISASM_LINES {
            let row = i + 1;
            let instr = disassemble_one(addr, &mut read_mem);
            let is_pc = addr == cpu.pc;
            let has_bp = state.breakpoints.breakpoints.iter()
                .any(|bp| bp.enabled && bp.kind == BreakpointKind::Opcode && bp.address == addr);
            let is_cursor = addr == state.cursor && !is_pc;

            // Choose line background
            let line_bg = if is_pc {
                BG_PC_LINE
            } else if has_bp {
                BG_BP_LINE
            } else if is_cursor {
                BG_CURSOR_LINE
            } else {
                BG_DEFAULT
            };

            // Fill entire line with background
            fill_row(fb, row, 0, INFO_COL, line_bg);

            // Marker column
            let marker = if is_pc { '>' } else if has_bp { '*' } else { ' ' };
            let marker_color = if is_pc { WHITE } else if has_bp { RED } else { GREY };
            draw_char(fb, 0, row, marker as u8, marker_color, line_bg);

            // Address
            let addr_str = format!("{:04X}", addr);
            draw_str(fb, 1, row, &addr_str, YELLOW, line_bg);
            draw_char(fb, 5, row, b':', GREY, line_bg);

            // Raw bytes
            let mut col = 6;
            for j in 0..instr.bytes {
                let b = read_mem(addr.wrapping_add(j as u16));
                let hex = format!("{:02X}", b);
                col = draw_str(fb, col, row, &hex, LT_BLUE, line_bg);
                col += 1;
            }
            // Pad to fixed column for mnemonic
            while col < 16 { col += 1; }

            // Mnemonic
            col = draw_str(fb, col, row, instr.mnemonic, WHITE, line_bg);
            col += 1;

            // Operand (from formatted instruction, skip "XXXX: MNE " prefix)
            let full = format_instruction(&instr);
            // full = "XXXX: MNE operand" — extract operand after mnemonic
            let after_mnemonic = if full.len() > 6 + instr.mnemonic.len() {
                &full[6 + instr.mnemonic.len()..]
            } else {
                ""
            };
            let operand = after_mnemonic.trim_start();
            if !operand.is_empty() {
                col = draw_str(fb, col, row, operand, CYAN, line_bg);
            }

            // Symbol annotation
            if let Some(name) = state.symbols.name_at(addr)
                && col < 40 {
                    col = draw_str(fb, col + 1, row, ";", GREY, line_bg);
                    draw_str(fb, col, row, name, GREEN, line_bg);
            }
            let _ = col; // suppress unused

            addr = addr.wrapping_add(instr.bytes as u16);
        }
    }

    // Vertical separator between disasm and info
    for row in 0..DATA_ROW {
        draw_char(fb, INFO_COL - 1, row, b'|', GREY, BG_DEFAULT);
    }

    // ════════════════════════════════════════════════════════════════════════
    // RIGHT: Registers (cols INFO_COL–79, rows 0–)
    // ════════════════════════════════════════════════════════════════════════
    let mut rrow = 0;
    {
        draw_str(fb, INFO_COL, rrow, " Registers", GREEN, BG_DEFAULT);
        rrow += 1;

        // PC
        let mut c = draw_str(fb, INFO_COL, rrow, "PC ", GREY, BG_DEFAULT);
        c = draw_str(fb, c, rrow, &format!("{:04X}", cpu.pc), ORANGE, BG_DEFAULT);
        c += 1;
        c = draw_str(fb, c, rrow, "A ", GREY, BG_DEFAULT);
        draw_str(fb, c, rrow, &format!("{:02X}", cpu.a), ORANGE, BG_DEFAULT);
        rrow += 1;

        // SP, X
        c = draw_str(fb, INFO_COL, rrow, "SP ", GREY, BG_DEFAULT);
        c = draw_str(fb, c, rrow, &format!("  {:02X}", cpu.sp), ORANGE, BG_DEFAULT);
        c += 1;
        c = draw_str(fb, c, rrow, "X ", GREY, BG_DEFAULT);
        draw_str(fb, c, rrow, &format!("{:02X}", cpu.x), ORANGE, BG_DEFAULT);
        rrow += 1;

        // P, Y
        c = draw_str(fb, INFO_COL, rrow, "P  ", GREY, BG_DEFAULT);
        c = draw_str(fb, c, rrow, &format!("  {:02X}", cpu.flags), ORANGE, BG_DEFAULT);
        c += 1;
        c = draw_str(fb, c, rrow, "Y ", GREY, BG_DEFAULT);
        draw_str(fb, c, rrow, &format!("{:02X}", cpu.y), ORANGE, BG_DEFAULT);
        rrow += 1;

        // Flags with set/clear visual
        c = draw_str(fb, INFO_COL, rrow, "  ", GREY, BG_DEFAULT);
        for &(bit, name) in &[(0x80u8, "N"), (0x40, "V"), (0x20, "-"), (0x10, "B"),
                               (0x08, "D"), (0x04, "I"), (0x02, "Z"), (0x01, "C")] {
            let set = cpu.flags & bit != 0;
            let (fg, bg) = if name == "-" {
                (GREY, BG_DEFAULT)
            } else if set {
                (BLACK, WHITE) // inverse video for set flags
            } else {
                (GREY, BG_DEFAULT)
            };
            let label = if set { name.to_uppercase() } else { name.to_lowercase() };
            c = draw_str(fb, c, rrow, &label, fg, bg);
        }
        rrow += 1;

        // Cycles
        draw_str(fb, INFO_COL, rrow, &format!("Cyc {}", cpu.cycles), GREY, BG_DEFAULT);
        rrow += 1;
    }

    // ── Stack ────────────────────────────────────────────────────────────────
    rrow += 1;
    {
        draw_str(fb, INFO_COL, rrow, " Stack", GREEN, BG_DEFAULT);
        rrow += 1;

        let top = cpu.sp.wrapping_add(1);
        let mut saddr = 0x0100u16 | top as u16;
        let mut count = 0;
        while saddr <= 0x01FF && count < 8 {
            let b = read_mem(saddr);
            let marker = if count == 0 { ">" } else { " " };
            draw_str(fb, INFO_COL, rrow, &format!(
                "{marker}{:04X}:{:02X}", saddr, b
            ), LT_BLUE, BG_DEFAULT);
            saddr += 1;
            count += 1;
            rrow += 1;
        }
        if count == 0 {
            draw_str(fb, INFO_COL, rrow, " (empty)", GREY, BG_DEFAULT);
            rrow += 1;
        }
    }

    // ── Breakpoints ──────────────────────────────────────────────────────────
    rrow += 1;
    {
        draw_str(fb, INFO_COL, rrow, " Breakpoints", GREEN, BG_DEFAULT);
        rrow += 1;

        if state.breakpoints.is_empty() {
            draw_str(fb, INFO_COL, rrow, " (none)", GREY, BG_DEFAULT);
            rrow += 1;
        } else {
            for (idx, bp) in state.breakpoints.breakpoints.iter().enumerate() {
                if rrow >= DATA_ROW - 2 { break; } // don't overflow into data window
                let kind = match bp.kind {
                    BreakpointKind::Opcode   => "PC",
                    BreakpointKind::MemRead  => "MR",
                    BreakpointKind::MemWrite => "MW",
                    _ => "??",
                };
                let en = if bp.enabled { " " } else { "D" };
                let color = if bp.enabled { RED } else { GREY };
                draw_str(fb, INFO_COL, rrow, &format!(
                    " {:X}:{en}${:04X} {kind}", idx, bp.address
                ), color, BG_DEFAULT);
                rrow += 1;
            }
        }
    }

    // ── Watches ──────────────────────────────────────────────────────────────
    if !state.watches.items.is_empty() && rrow < DATA_ROW - 2 {
        rrow += 1;
        draw_str(fb, INFO_COL, rrow, " Watches", GREEN, BG_DEFAULT);
        rrow += 1;
        for (idx, w) in state.watches.items.iter().enumerate() {
            if rrow >= DATA_ROW - 1 { break; }
            let b = read_mem(w.address);
            draw_str(fb, INFO_COL, rrow, &format!(
                " {:X}:${:04X}={:02X}", idx, w.address, b
            ), CYAN, BG_DEFAULT);
            rrow += 1;
        }
    }

    // ── Soft Switches (compact) ──────────────────────────────────────────────
    if rrow < DATA_ROW - 3 {
        rrow += 1;
        draw_str(fb, INFO_COL, rrow, " Switches", GREEN, BG_DEFAULT);
        rrow += 1;
        let info = decode_soft_switches(mode_bits);
        // Show in 2-column layout
        let items = &info.items;
        let mut i = 0;
        while i < items.len() && rrow < DATA_ROW - 1 {
            let sw1 = &items[i];
            let c1 = if sw1.active { GREEN } else { GREY };
            let s1 = if sw1.active { "+" } else { "-" };
            let c = draw_str(fb, INFO_COL, rrow, &format!("{s1}{:<9}", sw1.name), c1, BG_DEFAULT);
            i += 1;
            if i < items.len() {
                let sw2 = &items[i];
                let c2 = if sw2.active { GREEN } else { GREY };
                let s2 = if sw2.active { "+" } else { "-" };
                draw_str(fb, c + 1, rrow, &format!("{s2}{:<9}", sw2.name), c2, BG_DEFAULT);
                i += 1;
            }
            rrow += 1;
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    // DATA WINDOW: Memory hex dump (rows DATA_ROW–CONSOLE_ROW-1)
    // ════════════════════════════════════════════════════════════════════════
    draw_separator(fb, DATA_ROW, 0, COLS);
    {
        let c = draw_str(fb, 0, DATA_ROW, " Data ", GREEN, BG_DEFAULT);
        draw_str(fb, c, DATA_ROW, &format!("${:04X}", state.mem_view_addr), YELLOW, BG_DEFAULT);

        let mut addr = state.mem_view_addr;
        for i in 0..(CONSOLE_ROW - DATA_ROW - 1) {
            let row = DATA_ROW + 1 + i;

            // Address
            let mut c = draw_str(fb, 0, row, &format!("{:04X}:", addr), YELLOW, BG_DEFAULT);

            // Hex bytes
            for j in 0..8u16 {
                let b = read_mem(addr.wrapping_add(j));
                c = draw_str(fb, c, row, &format!("{:02X}", b), LT_BLUE, BG_DEFAULT);
                c += 1; // space
            }

            c += 1;

            // ASCII
            for j in 0..8u16 {
                let b = read_mem(addr.wrapping_add(j));
                let ch = if (0x20..=0x7E).contains(&b) { b } else { b'.' };
                c = draw_str(fb, c, row, &(ch as char).to_string(), CYAN, BG_DEFAULT);
            }

            addr = addr.wrapping_add(8);
        }
    }

    // ════════════════════════════════════════════════════════════════════════
    // CONSOLE: Output + command input (rows CONSOLE_ROW–47)
    // ════════════════════════════════════════════════════════════════════════
    draw_separator(fb, CONSOLE_ROW, 0, COLS);
    {
        // Show last few console lines
        let console_lines = ROWS - CONSOLE_ROW - 2; // leave 1 row for separator, 1 for input
        let output = &state.console_output;
        let start = output.len().saturating_sub(console_lines);
        for (i, line) in output[start..].iter().enumerate() {
            let row = CONSOLE_ROW + 1 + i;
            if row >= ROWS - 1 { break; }
            // Truncate to screen width
            let display: String = line.chars().take(COLS).collect();
            draw_str(fb, 0, row, &display, WHITE, BG_DEFAULT);
        }

        // Command input line (last row)
        let input_row = ROWS - 1;
        draw_char(fb, 0, input_row, b'>', ORANGE, BG_DEFAULT);
        let display_input: String = cmd_input.chars().take(COLS - 2).collect();
        let end = draw_str(fb, 1, input_row, &display_input, WHITE, BG_DEFAULT);
        // Blinking cursor
        draw_char(fb, end, input_row, b'_', WHITE, BG_DEFAULT);
    }
}
