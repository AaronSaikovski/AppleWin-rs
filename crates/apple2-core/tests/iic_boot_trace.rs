//! Boot regression tests for the Apple //c using the 32KB "3.5 ROM"
//! (ROM version 0, 342-0033-A) — the firmware embedded by the `applewin`
//! binary for the //c model.

use apple2_core::cards::disk2::Disk2Card;
use apple2_core::cards::mouse::MouseCard;
use apple2_core::cards::ssc::SscCard;
use apple2_core::emulator::Emulator;
use apple2_core::model::{Apple2Model, CpuType};

/// The 3.5 ROM (ROM 0, 342-0033-A) — same file `applewin` embeds for the //c.
const IIC_35_ROM: &str = "../../roms/Apple_IIc/Apple IIc ROM 00 - 342-0033-A - 1985.bin";
const DOS33: &str = "../../bin/DOS 3.3 System Master - 680-0210-A.dsk";
const PRODOS: &str = "../../bin/ProDOS_2_4_3.po";

/// Render text page 1 ($400-$7FF) into a single string (rows joined by '\n').
fn screen_text(emu: &Emulator) -> String {
    let mut out = String::new();
    for row in 0..24 {
        let base = 0x400 + ((row / 8) * 0x28) + ((row % 8) * 0x80);
        for col in 0..40 {
            let b = emu.bus.main_ram[base + col] & 0x7F;
            out.push(if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                ' '
            });
        }
        out.push('\n');
    }
    out
}

/// Build a //c emulator with the built-in peripheral layout used by the app
/// (serial ports in slots 1/2, mouse in slot 4, Disk II in slot 6).
fn make_iic(disk: Option<&str>) -> Emulator {
    let rom = std::fs::read(IIC_35_ROM).expect("3.5 ROM present");
    assert_eq!(rom.len(), 32768, "3.5 ROM must be 32KB");
    let mut emu = Emulator::new(rom, Apple2Model::AppleIIc, CpuType::Cpu65C02);
    emu.bus.cards.insert(Box::new(SscCard::new(1)));
    emu.bus.cards.insert(Box::new(SscCard::new(2)));
    emu.bus.cards.insert(Box::new(MouseCard::new(4)));
    let mut disk6 = Disk2Card::new(6);
    disk6.set_iwm(true); // //c drives its disk port through the internal IWM
    if let Some(d) = disk {
        let data = std::fs::read(d).expect("disk image present");
        let ext = d.rsplit('.').next().unwrap_or("dsk");
        assert!(disk6.load_drive(0, &data, ext), "disk image loaded");
    }
    emu.bus.cards.insert(Box::new(disk6));
    emu
}

/// Run the emulator in large cycle batches until `needle` appears on the text
/// screen or `cap_cycles` is reached, returning the final screen text.  Batching
/// (rather than one instruction per call) keeps the boot tests fast, and the
/// early-exit-on-match with a generous cap keeps them from being brittle to small
/// shifts in boot timing.
fn run_until(emu: &mut Emulator, needle: &str, cap_cycles: u64) -> String {
    const BATCH: u64 = 500_000;
    let mut ran = 0u64;
    while ran < cap_cycles {
        emu.execute(BATCH);
        ran += BATCH;
        let screen = screen_text(emu);
        if screen.contains(needle) {
            return screen;
        }
    }
    screen_text(emu)
}

/// The 3.5 ROM must reach the //c title banner shortly after power-on.
#[test]
fn iic_35rom_shows_title_banner() {
    let mut emu = make_iic(None);
    let screen = run_until(&mut emu, "Apple //c", 30_000_000);
    assert!(
        screen.contains("Apple //c"),
        "expected //c title banner, got:\n{screen}"
    );
}

/// The 3.5 ROM must boot the DOS 3.3 System Master from the Disk II in slot 6.
#[test]
fn iic_35rom_boots_dos33() {
    let mut emu = make_iic(Some(DOS33));
    let screen = run_until(&mut emu, "DOS VERSION 3.3", 150_000_000);
    assert!(
        screen.contains("DOS VERSION 3.3"),
        "expected DOS 3.3 greeting, got:\n{screen}"
    );
}

/// The 3.5 ROM must boot a ProDOS disk from the Disk II in slot 6.
///
/// Regression test for the //c IWM status-register poll: ProDOS turns the motor
/// off then polls the IWM status ($C0EE with Q6 high) for bit 5 to clear.  The
/// discrete Disk II model kept returning the stale latch (bit 5 set) during the
/// spin-down grace period, so ProDOS hung forever on the //c (it booted fine on
/// the //e, which does not force the internal ROM disk firmware).
#[test]
fn iic_35rom_boots_prodos() {
    let mut emu = make_iic(Some(PRODOS));
    let screen = run_until(&mut emu, "PRODOS", 150_000_000);
    assert!(
        screen.contains("PRODOS"),
        "expected ProDOS boot volume, got:\n{screen}"
    );
}
