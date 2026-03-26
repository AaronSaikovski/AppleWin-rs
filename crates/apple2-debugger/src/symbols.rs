//! Symbol table for the debugger.
//!
//! Reference: `source/Debugger/Debugger_Symbols.cpp`

use std::collections::HashMap;

/// A symbol table mapping addresses to names and vice versa.
#[derive(Debug, Default)]
pub struct SymbolTable {
    addr_to_name: HashMap<u16, String>,
    name_to_addr: HashMap<String, u16>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or update a symbol.
    pub fn insert(&mut self, name: impl Into<String>, addr: u16) {
        let name = name.into();
        self.addr_to_name.insert(addr, name.clone());
        self.name_to_addr.insert(name, addr);
    }

    /// Remove a symbol by name.
    pub fn remove_name(&mut self, name: &str) {
        if let Some(&addr) = self.name_to_addr.get(name) {
            self.addr_to_name.remove(&addr);
            self.name_to_addr.remove(name);
        }
    }

    /// Look up the name for an address, if any.
    pub fn name_at(&self, addr: u16) -> Option<&str> {
        self.addr_to_name.get(&addr).map(|s| s.as_str())
    }

    /// Look up the address for a name, if any.
    pub fn addr_of(&self, name: &str) -> Option<u16> {
        self.name_to_addr.get(name).copied()
    }
}
