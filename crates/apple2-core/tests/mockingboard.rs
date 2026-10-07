//! Mockingboard regression tests: the card is reached through its slot page
//! ($Cn00 / $Cn80, as real software and AppleWin expect) and its VIA timers
//! raise interrupts on their own, without the program touching the card.

use apple2_core::cards::mockingboard::MockingboardCard;
use apple2_core::emulator::Emulator;
use apple2_core::model::{Apple2Model, CpuType};

const SLOT: usize = 4;
const VIA1: u16 = 0xC400;
const VIA2: u16 = 0xC480;

/// 16K ROM of NOPs with reset → `$0300` and IRQ/BRK → `irq`.
fn rom(irq: u16) -> Vec<u8> {
    let mut rom = vec![0xEA; 16384];
    let v = |a: u16| (a - 0xC000) as usize;
    rom[v(0xFFFC)] = 0x00;
    rom[v(0xFFFD)] = 0x03;
    rom[v(0xFFFE)] = irq as u8;
    rom[v(0xFFFF)] = (irq >> 8) as u8;
    rom
}

fn emu_with_mb(program: &[u8], handler: &[u8]) -> Emulator {
    let mut emu = Emulator::new(rom(0x0340), Apple2Model::AppleIIeEnh, CpuType::Cpu65C02);
    emu.bus.cards.insert(Box::new(MockingboardCard::new(SLOT)));
    emu.bus.main_ram[0x0300..0x0300 + program.len()].copy_from_slice(program);
    emu.bus.main_ram[0x0340..0x0340 + handler.len()].copy_from_slice(handler);
    emu
}

fn wr(emu: &mut Emulator, addr: u16, val: u8) {
    let c = emu.cpu.cycles;
    emu.bus.write(addr, val, c);
}

fn rd(emu: &mut Emulator, addr: u16) -> u8 {
    let c = emu.cpu.cycles;
    emu.bus.read(addr, c)
}

/// Standard Mockingboard AY access: latch `reg`, write `val`, read it back.
fn ay_write_read(emu: &mut Emulator, via: u16, reg: u8, val: u8) -> u8 {
    wr(emu, via + 3, 0xFF); // DDRA out
    wr(emu, via + 2, 0x07); // DDRB out
    for (ora, orb) in [(reg, 0x07), (val, 0x06), (reg, 0x07)] {
        wr(emu, via + 1, ora);
        wr(emu, via, orb); // latch / write / latch
        wr(emu, via, 0x04); // inactive
    }
    wr(emu, via + 3, 0x00); // DDRA in
    wr(emu, via, 0x05); // read
    rd(emu, via + 1)
}

/// Arm T1 free-running with IRQ enabled and the given period (cycles).
fn arm_t1(emu: &mut Emulator, period: u16) {
    wr(emu, VIA1 + 0x0B, 0x40); // ACR: T1 continuous
    wr(emu, VIA1 + 0x0E, 0xC0); // IER: enable T1
    wr(emu, VIA1 + 0x04, period as u8);
    wr(emu, VIA1 + 0x05, (period >> 8) as u8); // T1CH: start
}

#[test]
fn ay_registers_are_reached_through_the_slot_page() {
    let mut emu = emu_with_mb(&[0x4C, 0x00, 0x03], &[]);
    assert_eq!(ay_write_read(&mut emu, VIA1, 2, 0x5A), 0x5A);
    assert_eq!(ay_write_read(&mut emu, VIA2, 2, 0xA5), 0xA5);
    // The two VIAs drive separate AY chips.
    assert_eq!(ay_write_read(&mut emu, VIA1, 3, 0x0F), 0x0F);
    wr(&mut emu, VIA1 + 1, 2);
    wr(&mut emu, VIA1, 0x07);
    wr(&mut emu, VIA1, 0x05);
    assert_eq!(
        rd(&mut emu, VIA1 + 1),
        0x5A,
        "VIA2 writes left AY A untouched"
    );
}

#[test]
fn via_interrupt_registers_are_addressable() {
    let mut emu = emu_with_mb(&[0x4C, 0x00, 0x03], &[]);
    wr(&mut emu, VIA1 + 0x0E, 0xC0);
    assert_eq!(rd(&mut emu, VIA1 + 0x0E), 0xC0, "IER at $Cn0E");
    wr(&mut emu, VIA2 + 0x0B, 0x40);
    assert_eq!(rd(&mut emu, VIA2 + 0x0B), 0x40, "ACR at $Cn8B");
    assert_eq!(rd(&mut emu, VIA1 + 0x0B), 0x00, "VIA1 ACR independent");
}

#[test]
fn timer_irq_asserts_without_the_program_touching_the_card() {
    // Interrupts stay masked (I set at reset): only the line is observed.
    let mut emu = emu_with_mb(&[0x4C, 0x00, 0x03], &[]);
    arm_t1(&mut emu, 1000);
    emu.execute(900);
    assert!(!emu.bus.irq_line, "not yet expired");
    emu.execute(300);
    assert!(emu.bus.irq_line, "T1 expiry raises IRQ on its own");
    rd(&mut emu, VIA1 + 0x04); // T1C-L read acknowledges
    assert!(!emu.bus.irq_line, "acknowledged");
}

#[test]
fn irq_driven_player_receives_every_timer_interrupt() {
    // Main: CLI ; JMP *       Handler: LDA $C404 ; INC $10 ; RTI
    let program = [0x58, 0x4C, 0x01, 0x03];
    let handler = [0xAD, 0x04, 0xC4, 0xE6, 0x10, 0x40];
    let mut emu = emu_with_mb(&program, &handler);
    arm_t1(&mut emu, 5000);
    emu.execute(50_000 + 1000);
    let ticks = emu.bus.main_ram[0x10];
    assert!(
        (9..=10).contains(&ticks),
        "expected ~10 interrupts in 10 periods, got {ticks}"
    );
}
