//! Watch point tracking.
//!
//! Watches are memory addresses whose values are displayed in the debugger UI
//! each time execution stops.

use serde::{Deserialize, Serialize};

/// A single watch point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watch {
    pub address: u16,
    pub length:  u16,
    pub label:   Option<String>,
}

/// Collection of watch points.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WatchManager {
    pub items: Vec<Watch>,
}

impl WatchManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, address: u16, length: u16) -> usize {
        self.items.push(Watch { address, length, label: None });
        self.items.len() - 1
    }

    pub fn add_labelled(&mut self, address: u16, length: u16, label: String) -> usize {
        self.items.push(Watch { address, length, label: Some(label) });
        self.items.len() - 1
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}
