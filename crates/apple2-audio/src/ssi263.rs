//! SSI-263A speech synthesis chip emulation.
//!
//! Used by Mockingboard and Phasor cards for phoneme-based speech.
//! Reference: `source/SSI263.cpp`

/// Number of phonemes in the SSI-263 phoneme set.
pub const NUM_PHONEMES: usize = 64;

/// SSI-263 register set.
#[derive(Debug, Default, Clone)]
pub struct Ssi263 {
    /// Phoneme / amplitude / rate register (reg 0).
    pub phoneme:   u8,
    /// Inflection register (reg 1).
    pub inflection: u8,
    /// Rate / speaking mode register (reg 2).
    pub rate:      u8,
    /// Control / articulation register (reg 3).
    pub control:   u8,
    /// Filter frequency register (reg 4).
    pub filter:    u8,

    /// True while a phoneme is being spoken.
    pub speaking:  bool,
    /// Pending IRQ flag.
    pub irq:       bool,
}

impl Ssi263 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Write to an SSI-263 register (address bits A2:A0 select register).
    pub fn write(&mut self, reg: u8, val: u8) {
        match reg & 0x07 {
            0 => {
                self.phoneme = val;
                self.speaking = true;
                // IRQ will fire when phoneme completes — simplified: immediate
                self.irq = true;
            }
            1 => self.inflection = val,
            2 => self.rate = val,
            3 => {
                self.control = val;
                if val & 0x40 != 0 {
                    // ACK clears IRQ
                    self.irq = false;
                }
            }
            4 => self.filter = val,
            _ => {}
        }
    }

    /// Read SSI-263 status.
    pub fn read(&self) -> u8 {
        // Bit 7: 1 = ready / not speaking
        if self.speaking { 0x00 } else { 0x80 }
    }

    /// Render phoneme audio into `out` (simplified stub — real synthesis is complex).
    pub fn render(&mut self, out: &mut [f32]) {
        if !self.speaking {
            return;
        }
        // Stub: produce silence; full implementation ports phoneme waveforms
        // from source/SSI263.cpp phoneme tables and formant synthesis.
        for s in out.iter_mut() {
            *s += 0.0;
        }
        self.speaking = false;
    }
}
