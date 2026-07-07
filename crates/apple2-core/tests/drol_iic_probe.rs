//! Divergence probe for the "Drol won't load on the //c" report.
//!
//! Boots the SAME disk image on an Apple //c and an Apple IIe Enhanced,
//! configured like the `applewin` binary, then compares where each machine
//! ends up: screen text, whether the disk motor is still spinning, and — when
//! a machine looks stuck — a PC histogram of the tight loop plus the $C0xx
//! soft switches it is polling.  This isolates the layer (disk load vs a
//! soft-switch / interrupt wait) before any fix is attempted.
//!
//! Run with the real image (not committed):
//!   DROL_IMAGE=/path/to/Drol.dsk cargo test -p apple2-core --test drol_iic_probe -- --nocapture --ignored
//!
//! `--ignored` because it needs an external image; it is skipped in CI.

use apple2_core::cards::disk2::Disk2Card;
use apple2_core::cards::mouse::MouseCard;
use apple2_core::cards::ssc::SscCard;
use apple2_core::emulator::Emulator;
use apple2_core::model::{Apple2Model, CpuType};
use std::collections::BTreeMap;

const IIC_35_ROM: &str = "../../roms/Apple_IIc/Apple IIc ROM 00 - 342-0033-A - 1985.bin";
const IIE_ROM: &str = "../../roms/apple2e_enhanced.rom";

fn screen_text(emu: &Emulator) -> String {
    let mut out = String::new();
    for row in 0..24 {
        let base = 0x400 + ((row / 8) * 0x28) + ((row % 8) * 0x80);
        for col in 0..40 {
            let b = emu.bus.main_ram[base + col] & 0x7F;
            out.push(if (0x20..0x7f).contains(&b) { b as char } else { ' ' });
        }
        out.push('\n');
    }
    out
}

fn load_image() -> (Vec<u8>, String) {
    let path = std::env::var("DROL_IMAGE").expect("set DROL_IMAGE=/path/to/Drol.dsk");
    let ext = path.rsplit('.').next().unwrap_or("dsk").to_lowercase();
    (std::fs::read(&path).expect("Drol image present"), ext)
}

/// Build a machine with the app's built-in //c layout, or a plain IIe with a
/// Disk II in slot 6.
fn make(model: Apple2Model, rom_path: &str, disk: &[u8], ext: &str) -> Emulator {
    let rom = std::fs::read(rom_path).expect("ROM present");
    let mut emu = Emulator::new(rom, model, CpuType::Cpu65C02);
    if model.is_iic() {
        emu.bus.cards.insert(Box::new(SscCard::new(1)));
        emu.bus.cards.insert(Box::new(SscCard::new(2)));
        emu.bus.cards.insert(Box::new(MouseCard::new(4)));
    }
    let mut disk6 = Disk2Card::new(6);
    if model.is_iic() {
        disk6.set_iwm(true);
    }
    assert!(disk6.load_drive(0, disk, ext), "disk image loaded into slot 6");
    emu.bus.cards.insert(Box::new(disk6));
    emu
}

/// Run in batches until the screen goes quiet (no change for `quiet_batches`
/// consecutive batches) or the cycle cap is hit. Returns (final_screen,
/// motor_on, total_cycles).
fn run_until_quiet(emu: &mut Emulator, cap_cycles: u64) -> (String, bool, u64) {
    const BATCH: u64 = 500_000;
    let mut ran = 0u64;
    let mut last = screen_text(emu);
    let mut quiet = 0;
    while ran < cap_cycles {
        emu.execute(BATCH);
        ran += BATCH;
        let now = screen_text(emu);
        if now == last {
            quiet += 1;
            if quiet >= 20 {
                break;
            }
        } else {
            quiet = 0;
            last = now;
        }
    }
    (screen_text(emu), emu.bus.disk_motor_on(), emu.cpu.cycles)
}

/// Single-step a window, building a PC histogram and a $C0xx access tally to
/// characterise the loop the machine is stuck in.
fn characterise_spin(emu: &mut Emulator, steps: u32) -> (Vec<(u16, u32)>, Vec<(u16, u32)>) {
    let mut pc_hist: BTreeMap<u16, u32> = BTreeMap::new();
    emu.bus.mem_trace_enabled = true;
    emu.bus.mem_trace.clear();
    for _ in 0..steps {
        *pc_hist.entry(emu.cpu.pc).or_default() += 1;
        emu.step();
    }
    emu.bus.mem_trace_enabled = false;

    // Tally $C000–$C0FF accesses captured in the trace (entry = (addr, val, is_read)).
    let mut io: BTreeMap<u16, u32> = BTreeMap::new();
    for &(addr, _val, _read) in emu.bus.mem_trace.iter() {
        if (0xC000..=0xC0FF).contains(&addr) {
            *io.entry(addr).or_default() += 1;
        }
    }
    let mut top_pc: Vec<_> = pc_hist.into_iter().collect();
    top_pc.sort_by(|a, b| b.1.cmp(&a.1));
    top_pc.truncate(12);
    let mut top_io: Vec<_> = io.into_iter().collect();
    top_io.sort_by(|a, b| b.1.cmp(&a.1));
    top_io.truncate(16);
    (top_pc, top_io)
}

fn report(label: &str, emu: &mut Emulator, cap: u64) {
    let (screen, motor, cyc) = run_until_quiet(emu, cap);
    println!("\n=== {label} ===");
    println!("cycles={cyc}  disk_motor_on={motor}");
    println!("--- screen ---\n{screen}");
    let (pcs, ios) = characterise_spin(emu, 200_000);
    println!("--- hottest PCs (spin loop) ---");
    for (pc, n) in &pcs {
        println!("  ${pc:04X}  x{n}");
    }
    println!("--- $C0xx soft switches polled in spin window ---");
    for (a, n) in &ios {
        println!("  ${a:04X}  x{n}");
    }
}

#[test]
#[ignore = "needs DROL_IMAGE env var pointing at a real disk image"]
fn drol_iic_vs_iie_divergence() {
    let (disk, ext) = load_image();
    const CAP: u64 = 200_000_000;

    let mut iic = make(Apple2Model::AppleIIc, IIC_35_ROM, &disk, &ext);
    report("Apple //c", &mut iic, CAP);

    let mut iie = make(Apple2Model::AppleIIeEnh, IIE_ROM, &disk, &ext);
    report("Apple IIe Enhanced", &mut iie, CAP);
}
