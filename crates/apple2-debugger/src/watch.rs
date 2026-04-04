//! Watch expressions for the debugger.
//!
//! Reference: `source/Debugger/Debugger_Types.h` (Watch_t)

use serde::{Deserialize, Serialize};

/// The source a watch reads from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WatchSource {
    /// Watch a single memory address (shows byte value).
    Address(u16),
    /// Watch a 16-bit word at address (little-endian).
    Word(u16),
    /// Watch a CPU register by name.
    Register(String),
}

/// A single watch entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watch {
    pub source: WatchSource,
    pub label:  Option<String>,
    pub enabled: bool,
}

/// Manages a list of watch expressions.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WatchManager {
    pub watches: Vec<Watch>,
}

impl WatchManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a watch and return its index.
    pub fn add(&mut self, watch: Watch) -> usize {
        self.watches.push(watch);
        self.watches.len() - 1
    }

    /// Remove a watch by index.
    pub fn remove(&mut self, index: usize) {
        if index < self.watches.len() {
            self.watches.remove(index);
        }
    }

    /// Clear all watches.
    pub fn clear(&mut self) {
        self.watches.clear();
    }

    /// Evaluate a watch source, returning a formatted string.
    ///
    /// `read` returns a byte from the address space.
    /// `reg` returns a register value by name (A, X, Y, SP, PC, P).
    pub fn evaluate<F, R>(source: &WatchSource, mut read: F, mut reg: R) -> String
    where
        F: FnMut(u16) -> u8,
        R: FnMut(&str) -> Option<u16>,
    {
        match source {
            WatchSource::Address(addr) => {
                let val = read(*addr);
                format!("${:04X} = {:02X} ({})", addr, val, val)
            }
            WatchSource::Word(addr) => {
                let lo = read(*addr) as u16;
                let hi = read(addr.wrapping_add(1)) as u16;
                let val = (hi << 8) | lo;
                format!("${:04X} = {:04X} ({})", addr, val, val)
            }
            WatchSource::Register(name) => {
                if let Some(val) = reg(name) {
                    format!("{} = {:04X} ({})", name, val, val)
                } else {
                    format!("{} = ???", name)
                }
            }
        }
    }
}
