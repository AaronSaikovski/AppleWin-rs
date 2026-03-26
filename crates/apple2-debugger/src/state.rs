//! Debugger state — owns all the globals that were spread across
//! 41 extern variables in `source/Debugger/Debug.h`.

use crate::breakpoint::BreakpointManager;
use crate::symbols::SymbolTable;

/// All mutable debugger state, in one owned struct.
#[derive(Debug, Default)]
pub struct DebuggerState {
    /// Active breakpoints.
    pub breakpoints: BreakpointManager,
    /// User-defined symbol table.
    pub symbols: SymbolTable,
    /// Current disassembly cursor address.
    pub cursor: u16,
    /// Step-over target address (used for "step over" / "next" command).
    pub step_over_target: Option<u16>,
    /// Whether the debugger is currently active (halting the emulator).
    pub active: bool,
    /// Last command string (for repeat on Enter).
    pub last_command: String,
}

impl DebuggerState {
    pub fn new() -> Self {
        Self::default()
    }
}
