//! Instruction trace buffer.
//!
//! Records the last N instructions executed for post-mortem analysis.

/// One trace entry.
#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub pc: u16,
    pub opcode: u8,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub flags: u8,
    pub cycles: u64,
    pub text: String,
}

/// Ring buffer of trace entries.
#[derive(Debug)]
pub struct TraceBuffer {
    entries: Vec<TraceEntry>,
    capacity: usize,
    write_pos: usize,
    count: usize,
    pub enabled: bool,
}

impl Default for TraceBuffer {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl TraceBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            capacity,
            write_pos: 0,
            count: 0,
            enabled: false,
        }
    }

    pub fn push(&mut self, entry: TraceEntry) {
        if !self.enabled {
            return;
        }
        if self.entries.len() < self.capacity {
            self.entries.push(entry);
        } else {
            self.entries[self.write_pos] = entry;
        }
        self.write_pos = (self.write_pos + 1) % self.capacity;
        self.count = (self.count + 1).min(self.capacity);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.write_pos = 0;
        self.count = 0;
    }

    /// Iterate entries from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = &TraceEntry> {
        let start = if self.count < self.capacity {
            0
        } else {
            self.write_pos
        };
        let len = self.count;
        (0..len).map(move |i| {
            let idx = (start + i) % self.entries.len();
            &self.entries[idx]
        })
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Return the last N entries (newest first).
    pub fn last_n(&self, n: usize) -> Vec<&TraceEntry> {
        let n = n.min(self.count);
        let mut result = Vec::with_capacity(n);
        for i in 0..n {
            let idx = if self.write_pos == 0 {
                self.entries.len() - 1 - i
            } else {
                (self.write_pos + self.entries.len() - 1 - i) % self.entries.len()
            };
            if idx < self.entries.len() {
                result.push(&self.entries[idx]);
            }
        }
        result
    }
}
