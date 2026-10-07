//! Debugger state — owns all the globals that were spread across
//! 41 extern variables in `source/Debugger/Debug.h`.

use crate::breakpoint::{BreakpointKind, BreakpointManager};
use crate::commands::CpuRegs;
use crate::disasm::{disassemble_one, format_instruction};
use crate::symbols::SymbolTable;
use crate::trace::{TraceBuffer, TraceEntry};
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

    /// Resume execution until the JSR about to execute returns to
    /// `return_pc` (the instruction after the JSR).
    pub fn begin_step_over(&mut self, return_pc: u16) {
        self.deactivate();
        self.step_over_target = Some(return_pc);
    }

    /// Resume execution until the current subroutine returns: the first
    /// RTS/RTI that pops the stack above `sp`.
    pub fn begin_step_out(&mut self, sp: u8) {
        self.deactivate();
        self.step_out_sp = Some(sp);
    }

    /// True while a step-over or step-out is running.
    pub fn is_running_to_target(&self) -> bool {
        self.step_over_target.is_some() || self.step_out_sp.is_some()
    }

    /// Resume execution for `n` instructions, recording each in the trace
    /// buffer, then halt (`T <n>`).
    pub fn begin_trace(&mut self, n: u32) {
        self.deactivate();
        if n > 0 {
            self.trace.enabled = true;
            self.trace_remaining = n;
        }
    }

    /// True when running code must be checked after every instruction
    /// (step-over/out, an active trace, or any enabled breakpoint).  When
    /// false the emulator runs its unhooked fast path.
    pub fn needs_instruction_hook(&self) -> bool {
        self.is_running_to_target()
            || self.trace_remaining > 0
            || self.breakpoints.breakpoints.iter().any(|bp| bp.enabled)
    }

    /// True when an enabled memory breakpoint exists, i.e. the bus must log
    /// accesses for [`after_instruction`](Self::after_instruction).
    pub fn needs_mem_trace(&self) -> bool {
        self.breakpoints.breakpoints.iter().any(|bp| {
            bp.enabled && matches!(bp.kind, BreakpointKind::MemRead | BreakpointKind::MemWrite)
        })
    }

    /// Debugger checks after one executed instruction.
    ///
    /// * `pc_before` — address the instruction was fetched from;
    /// * `regs` — CPU state after it (`regs.pc` is the next instruction);
    /// * `mem` — the instruction's bus accesses `(addr, value, is_write)`
    ///   (empty unless [`needs_mem_trace`](Self::needs_mem_trace));
    /// * `read` — side-effect-free memory read for disassembly.
    ///
    /// Records trace entries and halts (re-activating the debugger, returns
    /// `true`) on a step-over/out target, trace completion, a memory
    /// breakpoint hit by this instruction, or a PC breakpoint at the next one.
    pub fn after_instruction(
        &mut self,
        pc_before: u16,
        regs: CpuRegs,
        mem: &[(u16, u8, bool)],
        mut read: impl FnMut(u16) -> u8,
    ) -> bool {
        let opcode = read(pc_before);
        if self.check_step_target(opcode, regs.pc, regs.sp) {
            return true;
        }
        if self.trace_remaining > 0 {
            let instr = disassemble_one(pc_before, &mut read);
            self.trace.push(TraceEntry {
                pc: pc_before,
                opcode,
                a: regs.a,
                x: regs.x,
                y: regs.y,
                sp: regs.sp,
                flags: regs.flags,
                cycles: regs.cycles,
                text: format_instruction(&instr),
            });
            self.trace_remaining -= 1;
            if self.trace_remaining == 0 {
                self.activate(StopReason::TraceComplete);
                return true;
            }
        }
        for &(addr, _, is_write) in mem {
            if is_write && self.breakpoints.check_mem_write(addr) {
                self.activate(StopReason::MemWriteBreak(addr));
                return true;
            }
            if !is_write && self.breakpoints.check_mem_read(addr) {
                self.activate(StopReason::MemReadBreak(addr));
                return true;
            }
        }
        if self.breakpoints.check_opcode(regs.pc) {
            self.activate(StopReason::Breakpoint(regs.pc));
            return true;
        }
        false
    }

    /// Check a step-over/step-out after one instruction: `opcode` is the
    /// instruction just executed, `pc`/`sp` the CPU state after it.  When the
    /// target is reached the debugger re-activates (halting execution) and
    /// this returns `true`.
    pub fn check_step_target(&mut self, opcode: u8, pc: u16, sp: u8) -> bool {
        let reason = if self.step_over_target == Some(pc) {
            StopReason::StepOver
        } else if let Some(out_sp) = self.step_out_sp
            && matches!(opcode, 0x60 | 0x40) // RTS / RTI
            && sp > out_sp
        {
            StopReason::StepOut
        } else {
            return false;
        };
        self.step_over_target = None;
        self.step_out_sp = None;
        self.activate(reason);
        true
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::breakpoint::Breakpoint;

    #[test]
    fn step_over_runs_until_the_return_address() {
        let mut s = DebuggerState::new();
        s.begin_step_over(0x0303);
        assert!(!s.active && s.is_running_to_target());
        // Inside the subroutine: no stop.
        assert!(!s.check_step_target(0xA9, 0x1002, 0xFD));
        // RTS lands on the instruction after the JSR.
        assert!(s.check_step_target(0x60, 0x0303, 0xFF));
        assert!(s.active);
        assert_eq!(s.stop_reason, StopReason::StepOver);
        assert!(!s.is_running_to_target());
    }

    #[test]
    fn step_out_ignores_nested_returns() {
        let mut s = DebuggerState::new();
        s.begin_step_out(0xF0);
        // A nested JSR/RTS pair returns to the same stack level: keep going.
        assert!(!s.check_step_target(0x60, 0x2005, 0xF0));
        // Non-return instructions never stop, whatever the SP.
        assert!(!s.check_step_target(0x68, 0x2006, 0xF1)); // PLA
        // Our own RTS pops above the starting level.
        assert!(s.check_step_target(0x60, 0x0310, 0xF2));
        assert_eq!(s.stop_reason, StopReason::StepOut);
    }

    fn regs(pc: u16) -> CpuRegs {
        CpuRegs {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFF,
            pc,
            flags: 0,
            cycles: 0,
        }
    }

    const NOP: fn(u16) -> u8 = |_| 0xEA;

    #[test]
    fn pc_breakpoint_halts_before_the_instruction_at_its_address() {
        let mut s = DebuggerState::new();
        s.breakpoints.add(Breakpoint::at(0x0305));
        assert!(s.needs_instruction_hook());
        assert!(!s.needs_mem_trace(), "PC breakpoints need no bus logging");
        assert!(!s.after_instruction(0x0300, regs(0x0301), &[], NOP));
        assert!(s.after_instruction(0x0304, regs(0x0305), &[], NOP));
        assert!(s.active);
        assert_eq!(s.stop_reason, StopReason::Breakpoint(0x0305));
    }

    #[test]
    fn disabled_breakpoints_neither_hook_nor_fire() {
        let mut s = DebuggerState::new();
        let i = s.breakpoints.add(Breakpoint::at(0x0305));
        s.breakpoints.set_enabled(i, false);
        assert!(!s.needs_instruction_hook());
        assert!(!s.after_instruction(0x0304, regs(0x0305), &[], NOP));
    }

    #[test]
    fn memory_breakpoints_match_access_direction() {
        let mut s = DebuggerState::new();
        s.breakpoints.add(Breakpoint {
            kind: BreakpointKind::MemWrite,
            ..Breakpoint::at(0x2000)
        });
        assert!(s.needs_mem_trace());
        // A read of the watched address is not a write.
        assert!(!s.after_instruction(0x0300, regs(0x0303), &[(0x2000, 0, false)], NOP));
        assert!(s.after_instruction(0x0303, regs(0x0306), &[(0x2000, 7, true)], NOP));
        assert_eq!(s.stop_reason, StopReason::MemWriteBreak(0x2000));
    }

    #[test]
    fn trace_records_n_instructions_then_halts() {
        let mut s = DebuggerState::new();
        s.begin_trace(3);
        assert!(!s.active && s.needs_instruction_hook());
        assert!(!s.after_instruction(0x0300, regs(0x0301), &[], NOP));
        assert!(!s.after_instruction(0x0301, regs(0x0302), &[], NOP));
        assert!(s.after_instruction(0x0302, regs(0x0303), &[], NOP));
        assert_eq!(s.stop_reason, StopReason::TraceComplete);
        let pcs: Vec<u16> = s.trace.iter().map(|e| e.pc).collect();
        assert_eq!(pcs, vec![0x0300, 0x0301, 0x0302]);
        assert!(s.trace.iter().all(|e| e.text.contains("NOP")));
    }

    #[test]
    fn begin_survives_deactivation() {
        // Regression: the targets used to be set *before* `deactivate()`,
        // which cleared them, so step-over/out just resumed execution.
        let mut s = DebuggerState::new();
        s.activate(StopReason::UserBreak);
        s.begin_step_out(0xF0);
        assert_eq!(s.step_out_sp, Some(0xF0));
        s.begin_step_over(0x1234);
        assert_eq!(s.step_over_target, Some(0x1234));
    }
}
