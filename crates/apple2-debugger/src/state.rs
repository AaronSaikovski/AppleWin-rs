//! Debugger state — owns all the globals that were spread across
//! 41 extern variables in `source/Debugger/Debug.h`.

use crate::breakpoint::BreakpointManager;
use crate::symbols::SymbolTable;
use crate::trace::TraceBuffer;
use crate::watches::WatchManager;

/// All mutable debugger state, in one owned struct.
#[derive(Debug)]
pub struct DebuggerState {
    /// Active breakpoints.
    pub breakpoints: BreakpointManager,
    /// User-defined symbol table.
    pub symbols: SymbolTable,
    /// Watch points.
    pub watches: WatchManager,
    /// Instruction trace buffer.
    pub trace: TraceBuffer,
    /// Current disassembly cursor address.
    pub cursor: u16,
    /// Step-over target address (used for "step over" / "next" command).
    pub step_over_target: Option<u16>,
    /// Step-out target SP (run until RTS with SP >= this value).
    pub step_out_sp: Option<u8>,
    /// Whether the debugger is currently active (halting the emulator).
    pub active: bool,
    /// Last command string (for repeat on Enter).
    pub last_command: String,
    /// Command history.
    pub command_history: Vec<String>,
    /// Console output lines.
    pub console_output: Vec<String>,
    /// Current memory dump address (for scrolling through M command).
    pub mem_dump_addr: u16,
    /// Memory viewer start address (for the GUI hex viewer panel).
    pub mem_view_addr: u16,
    /// Number of trace instructions remaining.
    pub trace_remaining: u32,
    /// Reason the debugger last stopped.
    pub stop_reason: StopReason,
    /// Address the user is about to navigate to in disassembly.
    pub goto_addr: Option<u16>,
}

/// Reason the debugger halted execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// User requested pause.
    UserBreak,
    /// Hit a PC breakpoint.
    Breakpoint(u16),
    /// Hit a memory read breakpoint.
    MemReadBreak(u16),
    /// Hit a memory write breakpoint.
    MemWriteBreak(u16),
    /// Step completed.
    Step,
    /// Step-over completed.
    StepOver,
    /// Step-out completed.
    StepOut,
    /// Trace completed.
    TraceComplete,
    /// Not stopped (running).
    Running,
}

impl Default for DebuggerState {
    fn default() -> Self {
        Self {
            breakpoints: BreakpointManager::default(),
            symbols: SymbolTable::default(),
            watches: WatchManager::default(),
            trace: TraceBuffer::default(),
            cursor: 0,
            step_over_target: None,
            step_out_sp: None,
            active: false,
            last_command: String::new(),
            command_history: Vec::new(),
            console_output: Vec::new(),
            mem_dump_addr: 0,
            mem_view_addr: 0,
            trace_remaining: 0,
            stop_reason: StopReason::Running,
            goto_addr: None,
        }
    }
}

impl DebuggerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a line to the console output.
    pub fn print(&mut self, line: impl Into<String>) {
        self.console_output.push(line.into());
        // Keep last 500 lines
        if self.console_output.len() > 500 {
            self.console_output.drain(..self.console_output.len() - 500);
        }
    }

    /// Append multiple lines to the console output.
    pub fn print_lines(&mut self, lines: &[String]) {
        for line in lines {
            self.print(line.clone());
        }
    }

    /// Activate the debugger and halt execution.
    pub fn activate(&mut self, reason: StopReason) {
        self.active = true;
        self.stop_reason = reason;
    }

    /// Deactivate the debugger and resume execution.
    pub fn deactivate(&mut self) {
        self.active = false;
        self.step_over_target = None;
        self.step_out_sp = None;
        self.trace_remaining = 0;
        self.stop_reason = StopReason::Running;
    }

    /// Load standard Apple II symbols (common ROM entry points).
    pub fn load_apple2_symbols(&mut self) {
        let syms = [
            ("WNDLFT", 0x0020),
            ("WNDWDTH", 0x0021),
            ("WNDTOP", 0x0022),
            ("WNDBTM", 0x0023),
            ("CH", 0x0024),
            ("CV", 0x0025),
            ("BASL", 0x0028),
            ("BASH", 0x0029),
            ("BAS2L", 0x002A),
            ("BAS2H", 0x002B),
            ("INVFLAG", 0x0032),
            ("PROMPT", 0x0033),
            ("RESSION", 0x007B),
            ("HIMEM", 0x0073),
            ("LOMEM", 0x004A),
            // Monitor ROM
            ("HOME", 0xFC58),
            ("COUT", 0xFDED),
            ("COUT1", 0xFDF0),
            ("RDKEY", 0xFD0C),
            ("GETLN", 0xFD6A),
            ("GETLN1", 0xFD6F),
            ("CROUT", 0xFD8E),
            ("PRBYTE", 0xFDDA),
            ("PRHEX", 0xFDE3),
            ("BELL", 0xFF3A),
            ("VTAB", 0xFC22),
            ("CLREOL", 0xFC9C),
            ("CLREOP", 0xFC42),
            ("INIT", 0xFB2F),
            ("SETINV", 0xFE80),
            ("SETNORM", 0xFE84),
            ("RESET", 0xFA62),
            ("PWRUP", 0xFB60),
            // DOS 3.3
            ("RWTS", 0xB7B5),
        ];
        for (name, addr) in syms {
            self.symbols.insert(name, addr);
        }
    }
}
