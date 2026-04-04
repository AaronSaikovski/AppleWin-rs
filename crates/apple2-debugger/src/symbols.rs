//! Symbol table for the debugger.
//!
//! Reference: `source/Debugger/Debugger_Symbols.cpp`

use std::collections::HashMap;
use std::path::Path;

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

    /// Look up the address for a name, if any (case-insensitive).
    pub fn addr_of(&self, name: &str) -> Option<u16> {
        self.name_to_addr.get(name).copied()
    }

    /// Load symbols from a `.sym` file.
    ///
    /// Expected format: one symbol per line, `XXXX NAME` (hex address, whitespace, name).
    /// Lines starting with `;` or `#` are comments. Blank lines are skipped.
    /// Returns the number of symbols loaded.
    pub fn load_from_file(&mut self, path: &Path) -> Result<usize, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        let mut count = 0;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let addr_str = match parts.next() {
                Some(s) => s.trim_start_matches('$'),
                None => continue,
            };
            let name = match parts.next() {
                Some(s) => s,
                None => continue,
            };
            if let Ok(addr) = u16::from_str_radix(addr_str, 16) {
                self.insert(name, addr);
                count += 1;
            }
        }
        Ok(count)
    }

    /// Return the number of symbols in the table.
    pub fn len(&self) -> usize {
        self.addr_to_name.len()
    }

    /// Return true if the symbol table is empty.
    pub fn is_empty(&self) -> bool {
        self.addr_to_name.is_empty()
    }

    /// Iterate over all symbols as (address, name) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (u16, &str)> {
        self.addr_to_name.iter().map(|(&addr, name)| (addr, name.as_str()))
    }
}
