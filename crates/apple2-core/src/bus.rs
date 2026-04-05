//! Apple II memory bus.
//!
//! Replaces the global arrays `mem`, `memshadow[]`, `memwrite[]`,
//! `memreadPageType[]`, `IORead[256]`, `IOWrite[256]` from `source/Memory.h`.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use crate::card::{CardManager, DmaWrite};

// ── Memory mode flags ─────────────────────────────────────────────────────────

bitflags! {
    /// Memory soft-switch state (replaces `MF_*` defines in `source/Memory.h`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct MemMode: u32 {
        /// 80STORE: page 2 / hires use aux mem when PAGE2+HIRES set
        const MF_80STORE  = 0x0001;
        /// ALTZP: stack/zero-page in aux RAM
        const MF_ALTZP    = 0x0002;
        /// RAMRD: read from aux RAM ($0200–$BFFF)
        const MF_AUXREAD  = 0x0004;
        /// RAMWRT: write to aux RAM ($0200–$BFFF)
        const MF_AUXWRITE = 0x0008;
        /// Language Card: bank 2 ($D000–$DFFF)
        const MF_BANK2    = 0x0010;
        /// Language Card: RAM active ($D000–$FFFF)
        const MF_HIGHRAM  = 0x0020;
        /// HIRES: high-resolution graphics page
        const MF_HIRES    = 0x0040;
        /// PAGE2: alternate text/graphics page
        const MF_PAGE2    = 0x0080;
        /// SLOTC3ROM: slot 3 ROM visible
        const MF_SLOTC3ROM = 0x0100;
        /// INTCXROM: internal $C1–$CF ROM
        const MF_INTCXROM = 0x0200;
        /// Language Card: write-enabled
        const MF_WRITERAM = 0x0400;
        /// IOUDIS (Apple //c only)
        const MF_IOUDIS   = 0x0800;
        /// Alternate ROM bit 0
        const MF_ALTROM0  = 0x1000;
        /// Alternate ROM bit 1
        const MF_ALTROM1  = 0x2000;
        /// Graphics mode active ($C050 set, $C051 clear)
        const MF_GRAPHICS = 0x4000;
        /// Mixed mode — last 4 text rows overlay graphics ($C053)
        const MF_MIXED    = 0x8000;
        /// 80-column video mode — SET80VID ($C00D) / CLR80VID ($C00C)
        const MF_VID80    = 0x0001_0000;
        /// Alternate character set — SETALTCHAR ($C00F) / CLRALTCHAR ($C00E)
        const MF_ALTCHAR  = 0x0002_0000;
        /// Double hi-res / double lo-res enable — DHIRESON ($C05E) / DHIRESOFF ($C05F)
        const MF_DHIRES   = 0x0004_0000;
    }
}

impl Default for MemMode {
    fn default() -> Self {
        // Apple IIe power-on state: bank 2 selected, LC write-enabled.
        // Matches AppleWin C++ `kMemModeInitialState = MF_BANK2 | MF_WRITERAM`.
        MemMode::MF_BANK2 | MemMode::MF_WRITERAM
    }
}

// ── GamepadState ──────────────────────────────────────────────────────────────

/// Joystick / paddle state for the Apple II game I/O connector.
///
/// Updated by the host each frame based on keyboard, gamepad, or mouse input.
/// Drives the $C061–$C067 soft switches and the $C070 paddle one-shot timers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamepadState {
    /// Paddle 0 (X-axis): 0 = left/up, 255 = right/down, 127 = centre.
    pub paddle0: u8,
    /// Paddle 1 (Y-axis): 0 = up, 255 = down, 127 = centre.
    pub paddle1: u8,
    /// Button bitmask: bit 0 = Open Apple / btn 0, bit 1 = Closed Apple / btn 1,
    /// bit 2 = button 2 (rarely used).
    pub buttons: u8,
    /// CPU cycle at which the paddle-0 one-shot timer expires.
    paddle0_end: u64,
    /// CPU cycle at which the paddle-1 one-shot timer expires.
    paddle1_end: u64,
}

impl Default for GamepadState {
    fn default() -> Self {
        Self { paddle0: 127, paddle1: 127, buttons: 0, paddle0_end: 0, paddle1_end: 0 }
    }
}

impl GamepadState {
    /// Trigger the paddle one-shot timers (called on $C070 strobe).
    ///
    /// Timer duration = `value × 11 + 3` CPU cycles, matching the Apple II
    /// hardware spec (~11.149 µs per increment at 1.023 MHz).
    pub fn strobe(&mut self, cycles: u64) {
        self.paddle0_end = cycles + self.paddle0 as u64 * 11 + 3;
        self.paddle1_end = cycles + self.paddle1 as u64 * 11 + 3;
    }
}

// ── Page routing types ────────────────────────────────────────────────────────

/// Source for a 256-byte read page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSrc {
    /// Main RAM at given base address.
    Main(u16),
    /// Aux RAM at given base address.
    Aux(u16),
    /// ROM image at given base address.
    Rom(u16),
    /// I/O space — handled by card dispatch.
    Io,
    /// Floating bus (open bus return value).
    FloatingBus,
}

/// Destination for a 256-byte write page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageDst {
    /// Write to main RAM at given base address.
    Main(u16),
    /// Write to aux RAM at given base address.
    Aux(u16),
    /// Write inhibited (ROM, etc.).
    Inhibit,
}

// ── Bus ───────────────────────────────────────────────────────────────────────

/// The Apple II memory bus.
pub struct Bus {
    /// Main 64K RAM.
    pub main_ram: Box<[u8; 65536]>,
    /// Auxiliary 64K RAM (//e and later).
    pub aux_ram:  Box<[u8; 65536]>,
    /// System ROM (up to 16K for //e).
    pub rom:      Vec<u8>,
    /// Peripheral card ROM space ($C100–$CFFF, 3840 bytes = 15 pages).
    pub cx_rom:   Box<[u8; 0x1000]>,

    /// Memory mode soft switches.
    pub mode: MemMode,

    /// Per-page read routing (256 entries, one per 256-byte page).
    pub pages_r: [PageSrc; 256],
    /// Per-page write routing.
    pub pages_w: [PageDst; 256],

    /// Card manager — provides I/O dispatch and $Cn ROM images.
    pub cards: CardManager,

    /// Floating bus value (last byte on the bus — NTSC video data approximation).
    pub floating_bus: u8,

    /// Keyboard latch: bit 7 set = key available, bits 6–0 = ASCII.
    pub keyboard_data: u8,

    /// Current speaker cone position (toggled on each $C030 access).
    pub speaker_state: bool,
    /// Cycle timestamps of each speaker toggle, drained by the audio thread each frame.
    pub speaker_toggles: Vec<u64>,

    /// Joystick / gamepad state — updated by the host each frame.
    pub gamepad: GamepadState,

    /// RamWorks III: active auxiliary bank index (0 = primary aux_ram).
    pub rw3_active: u8,
    /// RamWorks III: extra aux banks (index 0 = bank 1, index 1 = bank 2, …).
    rw3_extra: Vec<Box<[u8; 65536]>>,

    /// Annunciator outputs 0–2 (annunciator 3 overlaps DHIRES at $C05E/$C05F).
    pub ann: [bool; 4],

    /// Reflects the current state of the card IRQ line (OR of all slots).
    pub irq_line: bool,

    /// Pre-allocated scratch buffer for Saturn LC bank swaps — eliminates a
    /// 16 KB heap allocation on every bank switch.
    lc_swap_buf: Box<[u8; 16384]>,

    /// Language Card: prewrite flag — set on first read of an odd LC switch.
    /// A second consecutive read of the same switch enables WRITERAM.
    lc_prewrite: bool,
    /// Language Card: last LC soft-switch register accessed (lower 4 bits).
    lc_last_access: u8,

    /// CPU cycle at which the current video frame began.
    ///
    /// Used to compute the within-frame cycle offset without a modulo operation:
    /// `frame_offset = cycles - frame_start_cycles`.  Updated lazily inside the
    /// `$C019` (VBLANK) soft-switch read and by `advance_frame()`.
    frame_start_cycles: u64,

    /// When true, record all memory accesses for debugger breakpoint inspection.
    /// Cost when false: one predicted branch per Bus::read/write (~0 overhead).
    pub mem_trace_enabled: bool,
    /// Per-instruction memory access log: (address, value, is_write).
    /// Drained after each instruction by `Emulator::execute_debugged`.
    pub mem_trace: Vec<(u16, u8, bool)>,

    // ── Cassette tape I/O ────────────────────────────────────────────────────

    /// Raw cassette audio data (8-bit unsigned PCM, ~11025 Hz).
    /// When `Some`, the $C060 soft switch returns bit 7 from this data stream.
    pub cassette_input: Option<Vec<u8>>,
    /// Current read position (byte index) in the cassette data.
    pub cassette_byte_pos: usize,
    /// CPU cycle at which cassette playback started — used to derive the
    /// current sample position from the cycle count.
    cassette_start_cycle: u64,
}

impl Bus {
    pub fn new(rom: Vec<u8>) -> Self {
        let mut bus = Self {
            main_ram:      Box::new([0u8; 65536]),
            aux_ram:       Box::new([0u8; 65536]),
            rom,
            cx_rom:        Box::new([0u8; 0x1000]),
            mode:          MemMode::default(),
            pages_r:       [PageSrc::Main(0); 256],
            pages_w:       [PageDst::Main(0); 256],
            cards:           CardManager::new(),
            floating_bus:    0,
            keyboard_data:   0,
            speaker_state:   false,
            speaker_toggles: Vec::with_capacity(65536),
            gamepad:         GamepadState::default(),
            rw3_active:      0,
            rw3_extra:          Vec::new(),
            ann:                [false; 4],
            irq_line:           false,
            lc_swap_buf:        Box::new([0u8; 16384]),
            lc_prewrite:        false,
            lc_last_access:     0,
            frame_start_cycles: 0,
            mem_trace_enabled:  false,
            mem_trace:          Vec::new(),
            cassette_input:       None,
            cassette_byte_pos:    0,
            cassette_start_cycle: 0,
        };
        bus.rebuild_page_tables();
        bus
    }

    /// Called by the host when a key is pressed.
    /// Sets the keyboard latch with strobe (bit 7 = 1).
    pub fn key_press(&mut self, ascii: u8) {
        self.keyboard_data = ascii | 0x80;
    }

    /// Notify the bus that a new video frame has begun at `cycles`.
    ///
    /// This updates `frame_start_cycles` so that the `$C019` (VBLANK) soft-switch
    /// read can compute the within-frame offset with a subtraction rather than a
    /// modulo.  Should be called from the emulator execute loop approximately
    /// every 17 030 CPU cycles (one NTSC frame).
    ///
    /// Calling this is optional — the VBLANK handler advances the counter lazily
    /// if `advance_frame` is never called — but calling it keeps the counter
    /// well-bounded and avoids drift over very long sessions.
    pub fn advance_frame(&mut self, cycles: u64) {
        const CYCLES_PER_FRAME: u64 = 65 * 262; // 17030
        // Only advance if we are actually past the end of the tracked frame.
        if cycles.wrapping_sub(self.frame_start_cycles) >= CYCLES_PER_FRAME {
            self.frame_start_cycles += CYCLES_PER_FRAME;
        }
    }

    /// Rebuild the page routing tables from the current `mode` state.
    ///
    /// Called after every soft-switch write, mirroring `MemUpdatePaging()`
    /// in `source/Memory.cpp`.
    pub fn rebuild_page_tables(&mut self) {
        let aux_read  = self.mode.contains(MemMode::MF_AUXREAD);
        let aux_write = self.mode.contains(MemMode::MF_AUXWRITE);
        let altzp     = self.mode.contains(MemMode::MF_ALTZP);
        let intcxrom  = self.mode.contains(MemMode::MF_INTCXROM);
        let highram   = self.mode.contains(MemMode::MF_HIGHRAM);
        let writeram  = self.mode.contains(MemMode::MF_WRITERAM);
        let bank2     = self.mode.contains(MemMode::MF_BANK2);

        // Pages 0x00–0x01: zero page + stack
        let zp_src = if altzp { PageSrc::Aux(0x0000) } else { PageSrc::Main(0x0000) };
        let zp_dst = if altzp { PageDst::Aux(0x0000) } else { PageDst::Main(0x0000) };
        self.pages_r[0x00] = zp_src;
        self.pages_r[0x01] = if altzp { PageSrc::Aux(0x0100) } else { PageSrc::Main(0x0100) };
        self.pages_w[0x00] = zp_dst;
        self.pages_w[0x01] = if altzp { PageDst::Aux(0x0100) } else { PageDst::Main(0x0100) };

        // Pages 0x02–0xBF: main/aux RAM
        for page in 0x02u16..=0xBF {
            let base = page << 8;
            self.pages_r[page as usize] = if aux_read {
                PageSrc::Aux(base)
            } else {
                PageSrc::Main(base)
            };
            self.pages_w[page as usize] = if aux_write {
                PageDst::Aux(base)
            } else {
                PageDst::Main(base)
            };
        }

        // 80STORE overrides: when active, PAGE2 routes display pages to aux
        if self.mode.contains(MemMode::MF_80STORE) {
            let page2 = self.mode.contains(MemMode::MF_PAGE2);
            if page2 {
                // Text page 1 ($0400–$07FF) → aux
                for page in 0x04u16..=0x07 {
                    let base = page << 8;
                    self.pages_r[page as usize] = PageSrc::Aux(base);
                    self.pages_w[page as usize] = PageDst::Aux(base);
                }
            }
            if page2 && self.mode.contains(MemMode::MF_HIRES) {
                // HiRes page 1 ($2000–$3FFF) → aux
                for page in 0x20u16..=0x3F {
                    let base = page << 8;
                    self.pages_r[page as usize] = PageSrc::Aux(base);
                    self.pages_w[page as usize] = PageDst::Aux(base);
                }
            }
        }

        // Page 0xC0: I/O
        self.pages_r[0xC0] = PageSrc::Io;
        self.pages_w[0xC0] = PageDst::Inhibit;

        // Pages 0xC1–0xCF: peripheral ROM or internal ROM
        let slotc3rom = self.mode.contains(MemMode::MF_SLOTC3ROM);
        for slot in 1u8..=0xF {
            let page = (0xC0 + slot) as usize;
            if intcxrom {
                // INTCXROM on: all slots read from internal ROM
                self.pages_r[page] = PageSrc::Rom(0xC000 + (slot as u16) * 0x100);
            } else if slot == 3 && !slotc3rom {
                // Apple IIe special: when SLOTC3ROM is off, $C300–$C3FF always
                // shows the internal 80-column firmware, not an external card.
                self.pages_r[page] = PageSrc::Rom(0xC300);
            } else {
                self.pages_r[page] = PageSrc::Io; // card Cx ROM — dispatched per read
            }
            self.pages_w[page] = PageDst::Inhibit;
        }

        // Pages 0xD0–0xDF: language card bank 1/2 or ROM
        // Bank2 lives in aux_ram[$D000-$DFFF].
        // Bank1 lives in aux_ram[$C000-$CFFF] (safe: that area is never used for normal RAM).
        for page in 0xD0u16..=0xDF {
            let base = page << 8;
            if highram {
                // LC RAM active — choose bank via MF_BANK2
                let lc_base = if bank2 { base } else { base - 0x1000 };
                self.pages_r[page as usize] = PageSrc::Aux(lc_base);
                self.pages_w[page as usize] = if writeram { PageDst::Aux(lc_base) } else { PageDst::Inhibit };
            } else {
                self.pages_r[page as usize] = PageSrc::Rom(base);
                self.pages_w[page as usize] = if writeram {
                    // Write to LC RAM even while ROM is being read (pre-condition for $C083 sequence)
                    let lc_base = if bank2 { base } else { base - 0x1000 };
                    PageDst::Aux(lc_base)
                } else {
                    PageDst::Inhibit
                };
            }
        }

        // Pages 0xE0–0xFF: upper ROM or LC RAM
        for page in 0xE0u16..=0xFF {
            let base = page << 8;
            if highram {
                self.pages_r[page as usize] = PageSrc::Aux(base);
                self.pages_w[page as usize] = if writeram { PageDst::Aux(base) } else { PageDst::Inhibit };
            } else {
                self.pages_r[page as usize] = PageSrc::Rom(base);
                // Write to LC RAM even while ROM is being read (same as $D000–$DFFF)
                self.pages_w[page as usize] = if writeram { PageDst::Aux(base) } else { PageDst::Inhibit };
            }
        }
    }

    // ── Aux RAM helpers ──────────────────────────────────────────────────────

    /// Read a byte from aux RAM, routing through the active RamWorks bank.
    /// The common case (`rw3_active == 0`) is a direct array access with no
    /// Vec overhead; the branch is almost always correctly predicted.
    #[inline(always)]
    fn aux_read(&self, idx: usize) -> u8 {
        if self.rw3_active == 0 {
            self.aux_ram[idx]
        } else {
            self.rw3_extra[(self.rw3_active as usize) - 1][idx]
        }
    }

    /// Write a byte to aux RAM, routing through the active RamWorks bank.
    #[inline(always)]
    fn aux_write(&mut self, idx: usize, val: u8) {
        if self.rw3_active == 0 {
            self.aux_ram[idx] = val;
        } else {
            self.rw3_extra[(self.rw3_active as usize) - 1][idx] = val;
        }
    }

    // ── Core read/write ──────────────────────────────────────────────────────

    /// Read a byte, triggering I/O side-effects (e.g. soft-switch reads).
    #[inline]
    pub fn read(&mut self, addr: u16, cycles: u64) -> u8 {
        let page = (addr >> 8) as usize;
        let val = match self.pages_r[page] {
            PageSrc::Main(base) => {
                self.main_ram[(base | (addr & 0xFF)) as usize]
            }
            PageSrc::Aux(base) => {
                self.aux_read((base | (addr & 0xFF)) as usize)
            }
            PageSrc::Rom(base) => {
                let rom_off = (base | (addr & 0xFF)) as usize;
                // ROM is mapped starting at $C000; offset accordingly
                let index = rom_off.saturating_sub(0xC000);
                self.rom.get(index).copied().unwrap_or(0)
            }
            PageSrc::Io => self.io_read(addr, cycles),
            PageSrc::FloatingBus => self.floating_bus,
        };
        if self.mem_trace_enabled {
            self.mem_trace.push((addr, val, false));
        }
        val
    }

    /// Write a byte, triggering I/O side-effects.
    #[inline]
    pub fn write(&mut self, addr: u16, val: u8, cycles: u64) {
        if self.mem_trace_enabled {
            self.mem_trace.push((addr, val, true));
        }
        let page = (addr >> 8) as usize;
        match self.pages_w[page] {
            PageDst::Main(base) => {
                self.main_ram[(base | (addr & 0xFF)) as usize] = val;
            }
            PageDst::Aux(base) => {
                self.aux_write((base | (addr & 0xFF)) as usize, val);
            }
            PageDst::Inhibit => {
                // ROM or I/O write that doesn't write RAM — still dispatch if I/O
                if page == 0xC0 || (0xC1..=0xCF).contains(&page) {
                    self.io_write(addr, val, cycles);
                }
            }
        }
    }

    /// Raw read that bypasses I/O side-effects.
    /// Used by the CPU reset vector fetch and debugger.
    #[inline]
    pub fn read_raw(&self, addr: u16) -> u8 {
        let page = (addr >> 8) as usize;
        match self.pages_r[page] {
            PageSrc::Main(base) => self.main_ram[(base | (addr & 0xFF)) as usize],
            PageSrc::Aux(base)  => {
                self.aux_read((base | (addr & 0xFF)) as usize)
            }
            PageSrc::Rom(base)  => {
                let index = ((base | (addr & 0xFF)) as usize).saturating_sub(0xC000);
                self.rom.get(index).copied().unwrap_or(0)
            }
            PageSrc::Io | PageSrc::FloatingBus => self.floating_bus,
        }
    }

    /// Raw write that bypasses I/O side-effects.
    #[inline]
    pub fn write_raw(&mut self, addr: u16, val: u8) {
        let page = (addr >> 8) as usize;
        match self.pages_w[page] {
            PageDst::Main(base) => self.main_ram[(base | (addr & 0xFF)) as usize] = val,
            PageDst::Aux(base)  => {
                self.aux_write((base | (addr & 0xFF)) as usize, val);
            }
            PageDst::Inhibit    => {}
        }
    }

    // ── I/O dispatch ($C000–$CFFF) ───────────────────────────────────────────

    fn io_read(&mut self, addr: u16, cycles: u64) -> u8 {
        let lo = addr & 0xFF;
        if addr < 0xC100 {
            // $C000–$C0FF: soft switches
            self.soft_switch_read(lo as u8, cycles)
        } else {
            // $C100–$CFFF: peripheral slot ROM
            let slot = ((addr >> 8) & 0xF) as usize;
            if let Some(card) = self.cards.slot_mut(slot) {
                card.io_read(lo as u8, cycles)
            } else {
                self.floating_bus
            }
        }
    }

    fn io_write(&mut self, addr: u16, val: u8, cycles: u64) {
        let lo = addr & 0xFF;
        if addr < 0xC100 {
            self.soft_switch_write(lo as u8, val, cycles);
        } else {
            let slot = ((addr >> 8) & 0xF) as usize;
            if let Some(card) = self.cards.slot_mut(slot) {
                card.io_write(lo as u8, val, cycles);
            }
        }
    }

    // ── Soft-switch dispatch ($C000–$C0FF) ───────────────────────────────────

    fn soft_switch_read(&mut self, reg: u8, cycles: u64) -> u8 {
        // $C000–$C0FF: Apple //e soft switches + slot peripheral I/O
        match reg {
            0x00 => self.keyboard_data,
            0x10 => {
                let old = self.keyboard_data;
                self.keyboard_data &= 0x7F;
                old
            }
            0x30 => {
                self.speaker_state = !self.speaker_state;
                self.speaker_toggles.push(cycles);
                self.floating_bus
            }
            0x11 => self.flag_byte(MemMode::MF_BANK2),
            0x12 => self.flag_byte(MemMode::MF_HIGHRAM),
            0x13 => self.flag_byte(MemMode::MF_AUXREAD),
            0x14 => self.flag_byte(MemMode::MF_AUXWRITE),
            0x15 => self.flag_byte(MemMode::MF_INTCXROM),
            0x16 => self.flag_byte(MemMode::MF_ALTZP),
            0x17 => self.flag_byte(MemMode::MF_SLOTC3ROM),
            0x18 => self.flag_byte(MemMode::MF_80STORE),
            // $C019: VBLANK bar — bit 7 = 1 during visible scan lines, 0 in blanking interval.
            // NTSC: 192 active lines × 65 CPU cycles/line = 12480; frame = 262 × 65 = 17030.
            // Matches AppleWin's NTSC_GetVblBar(): true when g_nVideoClockVert < 192.
            //
            // We avoid the expensive modulo by tracking `frame_start_cycles` and computing
            // the within-frame offset as a simple subtraction.  The frame boundary is
            // advanced lazily here; `advance_frame()` may also be called from the execute loop.
            0x19 => {
                const CYCLES_PER_FRAME: u64 = 65 * 262; // 17030
                const CYCLES_VISIBLE:   u64 = 65 * 192; // 12480
                let mut offset = cycles.wrapping_sub(self.frame_start_cycles);
                if offset >= CYCLES_PER_FRAME {
                    // Advance by whole frames so frame_start_cycles stays accurate even if
                    // advance_frame() was not called between frames.
                    let elapsed_frames = offset / CYCLES_PER_FRAME;
                    self.frame_start_cycles += elapsed_frames * CYCLES_PER_FRAME;
                    offset -= elapsed_frames * CYCLES_PER_FRAME;
                }
                if offset < CYCLES_VISIBLE { 0x80 } else { 0x00 }
            }
            // $C01A: RDTEXT — bit 7 = 1 when TEXT mode (graphics switch clear)
            0x1A => if !self.mode.contains(MemMode::MF_GRAPHICS) { 0x80 } else { 0x00 },
            // $C01B: RDMIXED — bit 7 = 1 when mixed mode
            0x1B => self.flag_byte(MemMode::MF_MIXED),
            0x1C => self.flag_byte(MemMode::MF_PAGE2),
            0x1D => self.flag_byte(MemMode::MF_HIRES),
            0x1E => self.flag_byte(MemMode::MF_ALTCHAR),
            0x1F => self.flag_byte(MemMode::MF_VID80),
            // $C061–$C063: game port buttons (bit 7 = pressed)
            0x61 => if self.gamepad.buttons & 0x01 != 0 { 0x80 } else { 0x00 },
            0x62 => if self.gamepad.buttons & 0x02 != 0 { 0x80 } else { 0x00 },
            0x63 => if self.gamepad.buttons & 0x04 != 0 { 0x80 } else { 0x00 },
            // $C064–$C067: paddle one-shot timers (bit 7 high until timer expires)
            0x64 => if cycles < self.gamepad.paddle0_end { 0x80 } else { 0x00 },
            0x65 => if cycles < self.gamepad.paddle1_end { 0x80 } else { 0x00 },
            0x66 | 0x67 => 0x00, // paddles 2/3 not connected
            // $C070: paddle strobe — resets timers and returns floating bus
            0x70 => { self.gamepad.strobe(cycles); self.floating_bus }
            // $C050–$C057: video soft-switch reads are strobes just like writes
            0x50 => { self.mode.insert(MemMode::MF_GRAPHICS);                      self.floating_bus }
            0x51 => { self.mode.remove(MemMode::MF_GRAPHICS);                      self.floating_bus }
            0x52 => { self.mode.remove(MemMode::MF_MIXED);                         self.floating_bus }
            0x53 => { self.mode.insert(MemMode::MF_MIXED);                         self.floating_bus }
            // $C054–$C057: PAGE2 / HIRES soft-switch reads act as strobes.
            // Only rebuild the page tables when the bit actually changes — programs
            // that poll these registers in tight loops would otherwise trigger a full
            // rebuild on every read even when the mode is unchanged.
            0x54 => {
                if self.mode.contains(MemMode::MF_PAGE2) {
                    self.mode.remove(MemMode::MF_PAGE2);
                    self.rebuild_page_tables();
                }
                self.floating_bus
            }
            0x55 => {
                if !self.mode.contains(MemMode::MF_PAGE2) {
                    self.mode.insert(MemMode::MF_PAGE2);
                    self.rebuild_page_tables();
                }
                self.floating_bus
            }
            0x56 => {
                if self.mode.contains(MemMode::MF_HIRES) {
                    self.mode.remove(MemMode::MF_HIRES);
                    self.rebuild_page_tables();
                }
                self.floating_bus
            }
            0x57 => {
                if !self.mode.contains(MemMode::MF_HIRES) {
                    self.mode.insert(MemMode::MF_HIRES);
                    self.rebuild_page_tables();
                }
                self.floating_bus
            }
            // $C058–$C05D: annunciators 0–2 (read-strobes, same as write)
            0x58 => { self.ann[0] = false; self.floating_bus }
            0x59 => { self.ann[0] = true;  self.floating_bus }
            0x5A => { self.ann[1] = false; self.floating_bus }
            0x5B => { self.ann[1] = true;  self.floating_bus }
            0x5C => { self.ann[2] = false; self.floating_bus }
            0x5D => { self.ann[2] = true;  self.floating_bus }
            // $C05E/$C05F: DHIRESON/DHIRESOFF — read also acts as write (same as $C050-$C057)
            0x5E => { self.mode.insert(MemMode::MF_DHIRES); self.floating_bus }
            0x5F => { self.mode.remove(MemMode::MF_DHIRES); self.floating_bus }
            // $C060: cassette input — bit 7 reflects the cassette audio waveform.
            // When no cassette is loaded, returns 0 (high-impedance / silence).
            0x60 => {
                if let Some(ref data) = self.cassette_input {
                    // Derive sample position from CPU cycles elapsed since playback
                    // started.  Cassette audio is 11025 Hz; CPU is ~1.023 MHz.
                    // sample = (cycles - start) * 11025 / 1023000
                    const CASSETTE_RATE: u64 = 11025;
                    const CPU_RATE: u64 = 1_023_000;
                    let elapsed = cycles.saturating_sub(self.cassette_start_cycle);
                    let sample_pos = (elapsed * CASSETTE_RATE / CPU_RATE) as usize;
                    self.cassette_byte_pos = sample_pos;
                    if sample_pos < data.len() {
                        // Unsigned 8-bit PCM: 128 = silence.  Return bit 7 based
                        // on whether the sample is above or below the midpoint.
                        if data[sample_pos] >= 128 { 0x80 } else { 0x00 }
                    } else {
                        0x00 // past end of tape
                    }
                } else {
                    0x00
                }
            }
            // $C07E: RDIOUDES — bit 7 = 1 when IOUDIS is set; $C07D: alternate read
            0x7D | 0x7E => self.flag_byte(MemMode::MF_IOUDIS),
            // $C07F: RDDHIRES — bit 7 = 1 when double hi-res is active
            0x7F => self.flag_byte(MemMode::MF_DHIRES),
            0x80..=0x8F => self.lc_read(reg),
            // $C090–$C0FF: peripheral card I/O (slots 1–7)
            // $C09x = slot 1, $C0Ax = slot 2, ..., $C0Ex = slot 6, $C0Fx = slot 7
            0x90..=0xFF => {
                let slot = ((reg as usize) >> 4) - 8; // 0x90>>4=9 → slot 1 .. 0xF0>>4=15 → slot 7
                let lo   = reg & 0x0F;
                if let Some(card) = self.cards.slot_mut(slot) {
                    let result = card.slot_io_read(lo, cycles);
                    self.process_card_dma(slot);
                    self.update_irq_line();
                    result
                } else {
                    self.floating_bus
                }
            }
            _ => self.floating_bus,
        }
    }

    fn soft_switch_write(&mut self, reg: u8, val: u8, cycles: u64) {
        match reg {
            0x00 => { self.mode.remove(MemMode::MF_80STORE); self.rebuild_page_tables(); }
            0x01 => { self.mode.insert(MemMode::MF_80STORE); self.rebuild_page_tables(); }
            // $C010: KBDSTRB — writing clears the keyboard strobe (same as reading it).
            // Many programs use STA $C010 rather than LDA $C010 to clear the strobe.
            0x10 => { self.keyboard_data &= 0x7F; }
            0x02 => { self.mode.remove(MemMode::MF_AUXREAD); self.rebuild_page_tables(); }
            0x03 => { self.mode.insert(MemMode::MF_AUXREAD); self.rebuild_page_tables(); }
            0x04 => { self.mode.remove(MemMode::MF_AUXWRITE); self.rebuild_page_tables(); }
            0x05 => { self.mode.insert(MemMode::MF_AUXWRITE); self.rebuild_page_tables(); }
            0x06 => { self.mode.remove(MemMode::MF_INTCXROM); self.rebuild_page_tables(); }
            0x07 => { self.mode.insert(MemMode::MF_INTCXROM); self.rebuild_page_tables(); }
            0x08 => { self.mode.remove(MemMode::MF_ALTZP); self.rebuild_page_tables(); }
            0x09 => { self.mode.insert(MemMode::MF_ALTZP); self.rebuild_page_tables(); }
            0x0A => { self.mode.remove(MemMode::MF_SLOTC3ROM); self.rebuild_page_tables(); }
            0x0B => { self.mode.insert(MemMode::MF_SLOTC3ROM); self.rebuild_page_tables(); }
            // $C00C/$C00D: CLR/SET80VID — 80-column display mode
            0x0C => { self.mode.remove(MemMode::MF_VID80); }
            0x0D => { self.mode.insert(MemMode::MF_VID80); }
            // $C00E/$C00F: CLRALTCHAR/SETALTCHAR — alternate character set
            0x0E => { self.mode.remove(MemMode::MF_ALTCHAR); }
            0x0F => { self.mode.insert(MemMode::MF_ALTCHAR); }
            // $C070: paddle strobe — reset one-shot timers
            0x70 => { self.gamepad.strobe(cycles); }
            // $C073: RamWorks III bank select
            0x73 => { self.rw3_switch(val); }
            0x30 => {
                self.speaker_state = !self.speaker_state;
                self.speaker_toggles.push(cycles);
            }
            // Text/graphics + mixed mode soft switches — video-only, no paging side-effects
            0x50 => { self.mode.insert(MemMode::MF_GRAPHICS); }
            0x51 => { self.mode.remove(MemMode::MF_GRAPHICS); }
            0x52 => { self.mode.remove(MemMode::MF_MIXED); }
            0x53 => { self.mode.insert(MemMode::MF_MIXED); }
            0x54 => { if  self.mode.contains(MemMode::MF_PAGE2) { self.mode.remove(MemMode::MF_PAGE2); self.rebuild_page_tables(); } }
            0x55 => { if !self.mode.contains(MemMode::MF_PAGE2) { self.mode.insert(MemMode::MF_PAGE2); self.rebuild_page_tables(); } }
            0x56 => { if  self.mode.contains(MemMode::MF_HIRES)  { self.mode.remove(MemMode::MF_HIRES);  self.rebuild_page_tables(); } }
            0x57 => { if !self.mode.contains(MemMode::MF_HIRES)  { self.mode.insert(MemMode::MF_HIRES);  self.rebuild_page_tables(); } }
            // $C058–$C05D: annunciators 0–2
            0x58 => { self.ann[0] = false; }
            0x59 => { self.ann[0] = true; }
            0x5A => { self.ann[1] = false; }
            0x5B => { self.ann[1] = true; }
            0x5C => { self.ann[2] = false; }
            0x5D => { self.ann[2] = true; }
            // $C05E/$C05F: DHIRESON/DHIRESOFF
            0x5E => { self.mode.insert(MemMode::MF_DHIRES); }
            0x5F => { self.mode.remove(MemMode::MF_DHIRES); }
            // $C07E: IOUDIS on; $C07F: IOUDIS off (in addition to DHIRESOFF read)
            0x7E => { self.mode.insert(MemMode::MF_IOUDIS); }
            0x7F => { self.mode.remove(MemMode::MF_IOUDIS); }
            0x80..=0x8F => self.lc_write(reg),
            0x90..=0xFF => {
                let slot = ((reg as usize) >> 4) - 8;
                let lo   = reg & 0x0F;
                if let Some(card) = self.cards.slot_mut(slot) {
                    card.slot_io_write(lo, val, cycles);
                    self.process_card_dma(slot);
                    self.process_lc_bank_swap(slot);
                    self.update_irq_line();
                }
            }
            _ => {}
        }
    }

    fn flag_byte(&self, flag: MemMode) -> u8 {
        if self.mode.contains(flag) { 0x80 } else { 0x00 }
    }

    /// Recompute `irq_line` by polling all cards for active IRQs.
    fn update_irq_line(&mut self) {
        self.irq_line = self.cards.any_irq_active();
    }

    /// Drain any pending DMA requests from a card and apply them to RAM.
    fn process_card_dma(&mut self, slot: usize) {
        // DMA write: card → main RAM
        if let Some(card) = self.cards.slot_mut(slot)
            && let Some(DmaWrite { dest, data }) = card.take_dma_write()
        {
            let dest = dest as usize;
            let end  = (dest + data.len()).min(65536);
            let len  = end - dest;
            self.main_ram[dest..end].copy_from_slice(&data[..len]);
        }
        // DMA read: main RAM → card (pass slice directly; no heap copy needed)
        if let Some(card) = self.cards.slot_mut(slot)
            && let Some((src, len)) = card.take_dma_read_request()
        {
            let src = src as usize;
            let len = len as usize;
            let end = (src + len).min(65536);
            card.dma_read_complete(&self.main_ram[src..end]);
        }
    }

    /// RamWorks III: switch to the given aux bank (0 = primary aux_ram).
    fn rw3_switch(&mut self, bank: u8) {
        if self.rw3_active == bank { return; }
        let needed = bank as usize;
        while self.rw3_extra.len() < needed {
            self.rw3_extra.push(Box::new([0u8; 65536]));
        }
        self.rw3_active = bank;
    }

    /// Perform a pending Saturn-style language card bank swap for `slot`.
    ///
    /// If the card in `slot` has a pending swap, the bus copies the new bank
    /// data into `aux_ram[$C000..]` and gives the displaced data back to the card.
    fn process_lc_bank_swap(&mut self, slot: usize) {
        let new_data = match self.cards.slot_mut(slot).and_then(|c| c.take_lc_bank_swap()) {
            Some(d) => d,
            None    => return,
        };
        // Save displaced bank into pre-allocated scratch buffer (no heap allocation).
        self.lc_swap_buf.copy_from_slice(&self.aux_ram[0xC000..0xC000 + 16384]);
        // Install the incoming bank.
        self.aux_ram[0xC000..0xC000 + 16384].copy_from_slice(&*new_data);
        // Return the displaced data to the card via a reference — the card copies
        // what it needs into its own storage, so no Box allocation is required here.
        if let Some(card) = self.cards.slot_mut(slot) {
            card.store_lc_bank(&self.lc_swap_buf);
        }
    }

    /// Language card soft-switch logic (simplified).
    /// Full implementation in `source/LanguageCard.cpp`.
    /// Language Card READ handler ($C080–$C08F).
    ///
    /// Implements the prewrite state machine matching real Apple II hardware:
    /// - Even addresses ($C080/2/4/6/8/A/C/E): clear prewrite, disable WRITERAM
    /// - Odd addresses ($C081/3/5/7/9/B/D/F): enable WRITERAM only after two
    ///   consecutive reads of the same register; first read sets prewrite flag
    fn lc_read(&mut self, reg: u8) -> u8 {
        let sel   = reg & 0x03;
        let bank2 = (reg & 0x08) == 0;
        let reg4  = reg & 0x0F;

        // WRITERAM logic: only odd-addressed reads can enable it
        if sel & 1 == 0 {
            // Even: clear prewrite, disable WRITERAM
            self.lc_prewrite = false;
            self.mode.remove(MemMode::MF_WRITERAM);
        } else {
            // Odd: need two consecutive reads of same register
            if self.lc_prewrite && self.lc_last_access == reg4 {
                self.mode.insert(MemMode::MF_WRITERAM);
            } else {
                self.lc_prewrite = true;
            }
        }

        // HIGHRAM (read-from-RAM vs read-from-ROM)
        match sel {
            0x00 | 0x03 => self.mode.insert(MemMode::MF_HIGHRAM),
            0x01 | 0x02 => self.mode.remove(MemMode::MF_HIGHRAM),
            _ => unreachable!(),
        }

        // Bank selection
        if bank2 { self.mode.insert(MemMode::MF_BANK2); }
        else     { self.mode.remove(MemMode::MF_BANK2); }

        self.lc_last_access = reg4;
        self.rebuild_page_tables();
        self.floating_bus
    }

    /// Language Card WRITE handler ($C080–$C08F).
    ///
    /// Writes always clear the prewrite flag and never enable WRITERAM.
    /// Even-addressed writes also disable WRITERAM; odd-addressed writes
    /// leave WRITERAM unchanged.  HIGHRAM and BANK2 update normally.
    fn lc_write(&mut self, reg: u8) {
        let sel   = reg & 0x03;
        let bank2 = (reg & 0x08) == 0;
        let reg4  = reg & 0x0F;

        // Any write clears prewrite
        self.lc_prewrite = false;

        // Writes NEVER enable WRITERAM; even writes also disable it
        if sel & 1 == 0 {
            self.mode.remove(MemMode::MF_WRITERAM);
        }
        // Odd writes: WRITERAM unchanged

        // HIGHRAM and BANK2 still update normally
        match sel {
            0x00 | 0x03 => self.mode.insert(MemMode::MF_HIGHRAM),
            0x01 | 0x02 => self.mode.remove(MemMode::MF_HIGHRAM),
            _ => unreachable!(),
        }

        if bank2 { self.mode.insert(MemMode::MF_BANK2); }
        else     { self.mode.remove(MemMode::MF_BANK2); }

        self.lc_last_access = reg4;
        self.rebuild_page_tables();
    }

    // ── Cassette helpers ──────────────────────────────────────────────────────

    /// Load raw unsigned 8-bit PCM cassette audio data (assumed 11025 Hz mono).
    ///
    /// Resets the playback position so the next $C060 read starts from the
    /// beginning of the data.  Pass the current CPU cycle count so the
    /// cycle-to-sample mapping starts correctly.
    pub fn load_cassette(&mut self, data: Vec<u8>, current_cycles: u64) {
        self.cassette_input = Some(data);
        self.cassette_byte_pos = 0;
        self.cassette_start_cycle = current_cycles;
    }

    /// Eject the cassette (stop providing data on $C060).
    pub fn eject_cassette(&mut self) {
        self.cassette_input = None;
        self.cassette_byte_pos = 0;
    }

    /// Returns true if a cassette tape is loaded.
    pub fn cassette_loaded(&self) -> bool {
        self.cassette_input.is_some()
    }

    // ── Disk helpers ─────────────────────────────────────────────────────────

    /// Returns true if any Disk II card in any slot currently has its motor on.
    pub fn disk_motor_on(&self) -> bool {
        (0..8).any(|s| self.cards.slot(s).is_some_and(|c| c.disk_motor_on()))
    }

    /// Load a disk image into the Disk2Card in `slot` (0–7), drive 0 or 1.
    /// Returns `true` if the card was found and accepted the image.
    pub fn load_disk(&mut self, slot: usize, drive: usize, data: &[u8], ext: &str) -> bool {
        use crate::cards::disk2::Disk2Card;
        if let Some(card) = self.cards.slot_mut(slot)
            && let Some(disk2) = card.as_any_mut().downcast_mut::<Disk2Card>()
        {
            return disk2.load_drive(drive, data, ext);
        }
        false
    }

    /// Set the file path for write-back on the Disk2Card in `slot`, drive 0 or 1.
    pub fn set_disk_path(&mut self, slot: usize, drive: usize, path: std::path::PathBuf) {
        use crate::cards::disk2::Disk2Card;
        if let Some(card) = self.cards.slot_mut(slot)
            && let Some(disk2) = card.as_any_mut().downcast_mut::<Disk2Card>()
        {
            disk2.set_drive_path(drive, path);
        }
    }

    /// Eject the disk in the Disk2Card in `slot`, drive 0 or 1.
    pub fn eject_disk(&mut self, slot: usize, drive: usize) {
        use crate::cards::disk2::Disk2Card;
        if let Some(card) = self.cards.slot_mut(slot)
            && let Some(disk2) = card.as_any_mut().downcast_mut::<Disk2Card>()
        {
            disk2.eject_drive(drive);
        }
    }
}

/// Snapshot of bus memory state for save states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusSnapshot {
    #[serde(with = "serde_bytes")]
    pub main_ram:     Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub aux_ram:      Vec<u8>,
    pub mode:         u32,
    pub speaker_state: bool,
}

impl Bus {
    pub fn take_snapshot(&self) -> BusSnapshot {
        BusSnapshot {
            main_ram:      self.main_ram.to_vec(),
            aux_ram:       self.aux_ram.to_vec(),
            mode:          self.mode.bits(),
            speaker_state: self.speaker_state,
        }
    }

    pub fn restore_snapshot(&mut self, snap: &BusSnapshot) {
        self.main_ram.copy_from_slice(&snap.main_ram);
        self.aux_ram.copy_from_slice(&snap.aux_ram);
        self.mode          = MemMode::from_bits_truncate(snap.mode);
        self.speaker_state = snap.speaker_state;
        self.rebuild_page_tables();
    }
}
