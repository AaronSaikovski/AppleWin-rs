//! Debugger state — owns all the globals that were spread across
//! 41 extern variables in `source/Debugger/Debug.h`.

use crate::breakpoint::BreakpointManager;
use crate::markup::MarkupMap;
use crate::symbols::SymbolTable;
use crate::watch::WatchManager;

/// All mutable debugger state, in one owned struct.
#[derive(Debug, Default)]
pub struct DebuggerState {
    /// Active breakpoints.
    pub breakpoints: BreakpointManager,
    /// User-defined symbol table.
    pub symbols: SymbolTable,
    /// Watch expressions.
    pub watches: WatchManager,
    /// Data/code markup regions.
    pub markup: MarkupMap,
    /// Current disassembly cursor address.
    pub cursor: u16,
    /// Step-over target address (used for "step over" / "next" command).
    pub step_over_target: Option<u16>,
    /// Whether the debugger is currently active (halting the emulator).
    pub active: bool,
    /// Last command string (for repeat on Enter).
    pub last_command: String,
    /// Command history (most recent last).
    pub command_history: Vec<String>,
    /// Console output lines.
    pub console_output: Vec<String>,
    /// Memory dump start address.
    pub mem_dump_addr: u16,
    /// Cycle counter checkpoint — cycles at last reset.
    pub cycle_checkpoint: u64,
}

impl DebuggerState {
    pub fn new() -> Self {
        Self::default()
    }
}
