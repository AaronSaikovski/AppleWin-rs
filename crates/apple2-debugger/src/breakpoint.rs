//! Breakpoint types and management.
//!
//! Ports the 13 breakpoint kinds from `source/Debugger/Debug.h`.

use serde::{Deserialize, Serialize};

/// The kind of breakpoint condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BreakpointKind {
    /// Break on PC reaching an address.
    Opcode,
    /// Break when a register equals a value.
    Register,
    /// Break on memory read from address.
    MemRead,
    /// Break on memory write to address.
    MemWrite,
    /// Break on IRQ/NMI.
    Interrupt,
    /// Break when video raster reaches a position.
    VideoPos,
    /// Break on I/O read.
    IoRead,
    /// Break on I/O write.
    IoWrite,
    /// Break after N instructions.
    Countdown,
    /// Break on expression evaluate to true.
    Expression,
}

/// A single breakpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breakpoint {
    pub kind:    BreakpointKind,
    pub address: u16,
    pub length:  u16,  // address range length (1 = single address)
    pub enabled: bool,
    pub label:   Option<String>,
}

impl Breakpoint {
    pub fn at(addr: u16) -> Self {
        Self {
            kind:    BreakpointKind::Opcode,
            address: addr,
            length:  1,
            enabled: true,
            label:   None,
        }
    }
}

/// Collection of active breakpoints.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BreakpointManager {
    pub breakpoints: Vec<Breakpoint>,
}

impl BreakpointManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, bp: Breakpoint) -> usize {
        self.breakpoints.push(bp);
        self.breakpoints.len() - 1
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.breakpoints.len() {
            self.breakpoints.remove(index);
        }
    }

    /// Check whether execution at `pc` should break.
    pub fn check_opcode(&self, pc: u16) -> bool {
        self.breakpoints.iter().any(|bp| {
            bp.enabled
                && bp.kind == BreakpointKind::Opcode
                && pc >= bp.address
                && pc < bp.address.saturating_add(bp.length)
        })
    }

    /// Check whether a memory read at `addr` should break.
    pub fn check_mem_read(&self, addr: u16) -> bool {
        self.breakpoints.iter().any(|bp| {
            bp.enabled
                && bp.kind == BreakpointKind::MemRead
                && addr >= bp.address
                && addr < bp.address.saturating_add(bp.length)
        })
    }

    /// Check whether a memory write at `addr` should break.
    pub fn check_mem_write(&self, addr: u16) -> bool {
        self.breakpoints.iter().any(|bp| {
            bp.enabled
                && bp.kind == BreakpointKind::MemWrite
                && addr >= bp.address
                && addr < bp.address.saturating_add(bp.length)
        })
    }

    /// Returns true if any enabled opcode breakpoints exist.
    pub fn has_opcode_breakpoints(&self) -> bool {
        self.breakpoints.iter().any(|bp| bp.enabled && bp.kind == BreakpointKind::Opcode)
    }

    /// Returns true if any enabled memory read/write breakpoints exist.
    pub fn has_mem_breakpoints(&self) -> bool {
        self.breakpoints.iter().any(|bp| {
            bp.enabled && matches!(bp.kind, BreakpointKind::MemRead | BreakpointKind::MemWrite)
        })
    }

    /// Returns true if any enabled breakpoints of any kind exist.
    pub fn has_any_breakpoints(&self) -> bool {
        self.breakpoints.iter().any(|bp| bp.enabled)
    }

    /// Check memory accesses from a trace slice for read/write breakpoints.
    pub fn check_mem_trace(&self, trace: &[(u16, u8, bool)]) -> bool {
        trace.iter().any(|&(addr, _, is_write)| {
            if is_write {
                self.check_mem_write(addr)
            } else {
                self.check_mem_read(addr)
            }
        })
    }
}
