use apple2_core::{
    emulator::Emulator,
    model::{Apple2Model, CpuType},
};

/// Apple IIe Enhanced 16KB ROM embedded at compile time.
static APPLE2E_ROM: &[u8] = include_bytes!("../../../roms/apple2e_enhanced.rom");

/// Apple IIc 32KB ROM (ROM version 0 — the "3.5 ROM", 342-0033-A) embedded at
/// compile time.  This is the first 32KB IIc firmware, adding UniDisk 3.5
/// support, the Mini-Assembler, and the self-test diagnostic.  It is a genuine
/// dual-bank ROM: the lower 16KB is the standard bank (active at power-on) and
/// the upper 16KB is the alternate bank selected via the $C028 ROM switch.
static APPLE2C_ROM: &[u8] =
    include_bytes!("../../../roms/Apple_IIc/Apple IIc ROM 00 - 342-0033-A - 1985.bin");

#[cfg(feature = "gui")]
mod config;

/// Wrapper enum for either the Apple IIe or Apple IIgs emulator core.
#[cfg(feature = "gui")]
#[allow(clippy::large_enum_variant)]
enum EmuCore {
    /// Apple II / IIe / IIe Enhanced / IIc emulation (6502/65C02).
    IIe(Emulator),
    /// Apple IIgs emulation (65C816).
    IIgs(Box<apple2_iigs::emulator::IIgsEmulator>),
}

fn make_emulator(
    machine: Apple2Model,
    cpu: CpuType,
    custom_rom: &Option<String>,
    custom_f8_rom: &Option<String>,
) -> Emulator {
    // Apple IIc always uses a 65C02.
    let cpu = if machine.is_iic() {
        CpuType::Cpu65C02
    } else {
        cpu
    };

    // Select the correct default ROM for the machine model.
    let default_rom: &[u8] = if machine.is_iic() {
        APPLE2C_ROM
    } else {
        APPLE2E_ROM
    };

    let mut rom = if let Some(path) = custom_rom {
        match std::fs::read(path) {
            Ok(data) if data.len() == 16384 || data.len() == 12288 || data.len() == 32768 => {
                // Pad 12K ROMs to 16K (add 4K of 0xFF at the start).
                if data.len() == 12288 {
                    let mut padded = vec![0xFF; 4096];
                    padded.extend_from_slice(&data);
                    padded
                } else {
                    data
                }
            }
            Ok(data) => {
                eprintln!(
                    "Custom ROM wrong size ({} bytes, expected 12K, 16K, or 32K), using default",
                    data.len()
                );
                default_rom.to_vec()
            }
            Err(e) => {
                eprintln!("Failed to load custom ROM '{}': {}, using default", path, e);
                default_rom.to_vec()
            }
        }
    } else {
        default_rom.to_vec()
    };

    // IIc 32K ROM: if the alternate bank (upper 16K) is empty, mirror the
    // standard bank (lower 16K) into it.  Some ROM dumps are 16K padded to
    // 32K; without mirroring, the $C028 ROM bank switch jumps into zeros.
    if machine.is_iic() && rom.len() == 32768 {
        let upper_empty = rom[0x4000..].iter().all(|&b| b == 0);
        if upper_empty {
            let lower: Vec<u8> = rom[..0x4000].to_vec();
            rom[0x4000..].copy_from_slice(&lower);
        }
    }

    // Patch $F800–$FFFF with a custom F8 ROM (2K) if configured.
    if let Some(path) = custom_f8_rom {
        match std::fs::read(path) {
            Ok(data) if data.len() == 2048 => {
                // F8 ROM goes at offset 0x3800 in the 16K ROM image
                // ($F800 - $C000 = $3800 = 14336).
                // For 32K IIc ROMs, patch the standard bank (upper 16K).
                let offset = if rom.len() > 16384 { 0x7800 } else { 0x3800 };
                if rom.len() >= offset + 2048 {
                    rom[offset..offset + 2048].copy_from_slice(&data);
                }
            }
            Ok(data) => {
                eprintln!(
                    "Custom F8 ROM wrong size ({} bytes, expected 2K)",
                    data.len()
                );
            }
            Err(e) => {
                eprintln!("Failed to load custom F8 ROM '{}': {}", path, e);
            }
        }
    }

    Emulator::new(rom, machine, cpu)
    // Card insertion is handled by apply_slot_cards() in the gui module.
}

/// Default IIgs ROM search paths (checked in order).
/// The latest ROM (ROM 03, 256KB) is preferred.
#[cfg(feature = "gui")]
fn find_iigs_rom(configured_path: &Option<String>) -> Option<Vec<u8>> {
    // If user configured an explicit path, try that first
    if let Some(path) = configured_path {
        match std::fs::read(path) {
            Ok(data) if data.len() == 131072 || data.len() == 262144 => return Some(data),
            Ok(data) => {
                eprintln!(
                    "IIgs ROM wrong size ({} bytes, expected 128K or 256K): {}",
                    data.len(),
                    path
                );
            }
            Err(e) => {
                eprintln!("Failed to load IIgs ROM '{}': {}", path, e);
            }
        }
    }

    // Search common locations for IIgs ROMs. Prefer ROM 01 (342-0077-B): it is
    // the standard, most widely compatible image and the default target of every
    // major IIgs emulator. ROM 00 is the fallback; the various ROM 3 dumps
    // (Tenspeed/BT/Alpha/Mark Twain) are non-standard collector images.
    let search_names = [
        // ROM 01 (128KB) — standard, most compatible.
        "Apple IIGS ROM 01 - 342-0077-B.bin",
        "Apple IIgs ROM1 - 342-0077-B -  27C1001.bin",
        // ROM 00 (128KB) — original.
        "Apple IIGS ROM 00 - 342-0077-A.bin",
        // ROM 3 (256KB, combined image) — later 2 MB machine.
        "Apple IIGS ROM 3 Tenspeed Late 1988 Early 1989 v25.bin",
        "Apple IIGS ROM 3 Tenspeed Late 1988 Early 1989 v16.bin",
    ];

    // Search directories relative to the executable and in common locations
    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();

    // Next to the executable
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        search_dirs.push(dir.join("roms"));
        search_dirs.push(dir.join("roms/Apple_IIgs"));
        search_dirs.push(dir.to_path_buf());
    }

    // Current working directory
    if let Ok(cwd) = std::env::current_dir() {
        search_dirs.push(cwd.join("roms"));
        search_dirs.push(cwd.join("roms/Apple_IIgs"));
        search_dirs.push(cwd.clone());
    }

    for dir in &search_dirs {
        for name in &search_names {
            let path = dir.join(name);
            if let Ok(data) = std::fs::read(&path)
                && (data.len() == 131072 || data.len() == 262144)
            {
                println!("Found IIgs ROM: {}", path.display());
                return Some(data);
            }
        }
    }

    None
}

/// Create an IIgs emulator, loading the ROM from file.
#[cfg(feature = "gui")]
fn make_iigs_emulator(
    iigs_rom_path: &Option<String>,
    iigs_ram_kb: u32,
) -> Result<apple2_iigs::emulator::IIgsEmulator, String> {
    let rom_data = find_iigs_rom(iigs_rom_path).ok_or_else(|| {
        "Apple IIgs ROM not found. Place a ROM file (ROM 01 or ROM 03) in the \
         'roms/Apple_IIgs/' directory next to the executable, or set the \
         'iigs_rom_path' in config.toml."
            .to_string()
    })?;

    apple2_iigs::emulator::IIgsEmulator::new(iigs_ram_kb as usize, rom_data)
}

fn main() {
    println!("AppleWin-rs v{}", env!("CARGO_PKG_VERSION"));

    #[cfg(feature = "gui")]
    {
        let cfg = config::Config::load();

        if cfg.machine_type == Apple2Model::AppleIIgs {
            // Apple IIgs mode
            match make_iigs_emulator(&cfg.iigs_rom_path, cfg.iigs_ram_kb) {
                Ok(iigs_emu) => {
                    println!(
                        "IIgs emulator initialised — ROM {:?}  PC=${:04X}  PBR=${:02X}",
                        iigs_emu.bus.mem.rom_version, iigs_emu.cpu.pc, iigs_emu.cpu.pbr
                    );
                    let core = EmuCore::IIgs(Box::new(iigs_emu));
                    gui::run_with_core(core, cfg);
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    eprintln!("Falling back to Apple IIe Enhanced");
                    let emu =
                        make_emulator(Apple2Model::AppleIIeEnh, CpuType::Cpu65C02, &None, &None);
                    let core = EmuCore::IIe(emu);
                    gui::run_with_core(core, cfg);
                }
            }
        } else {
            let emu = make_emulator(
                cfg.machine_type,
                cfg.cpu_type,
                &cfg.custom_rom_path,
                &cfg.custom_f8_rom_path,
            );
            println!(
                "Emulator initialised — model={:?}  PC=${:04X}",
                emu.model, emu.cpu.pc
            );
            let core = EmuCore::IIe(emu);
            gui::run_with_core(core, cfg);
        }
    }

    #[cfg(not(feature = "gui"))]
    {
        let mut emu = make_emulator(Apple2Model::AppleIIeEnh, CpuType::Cpu65C02, &None, &None);
        emu.mode = apple2_core::emulator::AppMode::Running;
        headless::run(&mut emu);
    }
}

// ── Headless ─────────────────────────────────────────────────────────────────

#[cfg(not(feature = "gui"))]
mod headless {
    use apple2_core::emulator::Emulator;
    pub fn run(emu: &mut Emulator) {
        const ONE_SECOND: u64 = 1_023_000;
        let executed = emu.execute(ONE_SECOND);
        println!(
            "Headless — executed {} cycles, PC=${:04X}",
            executed, emu.cpu.pc
        );
    }
}

// ── GUI (eframe 0.23 + egui 0.23) ────────────────────────────────────────────

#[cfg(feature = "gui")]
mod gui;
