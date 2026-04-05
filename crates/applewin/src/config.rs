/// Persistent configuration for AppleWin-rs.
///
/// Stored as TOML in the OS application-data directory:
///   Windows : %APPDATA%\applewin-rs\config.toml
///   macOS   : ~/Library/Application Support/applewin-rs/config.toml
///   Linux   : $XDG_CONFIG_HOME/applewin-rs/config.toml  (or ~/.config/…)
use apple2_core::{card::CardType, model::{Apple2Model, CpuType}};
use std::path::PathBuf;

// ── VideoType ─────────────────────────────────────────────────────────────────

/// Video emulation mode, matching AppleWin's `VideoType_e` / REGVALUE_VIDEO_MODE.
///
/// Default is `ColorTV` (VT_COLOR_TV = 4), matching AppleWin's out-of-box setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum VideoType {
    /// Custom monochrome colour (use `monochrome_color` field).
    MonoCustom      = 0,
    /// Color (Composite Idealized) — simplified NTSC colour-cell rendering.
    ColorIdealized  = 1,
    /// Color RGB Card / Monitor (future: RGB card output).
    ColorRGB        = 2,
    /// Color Monitor NTSC — NTSC signal-chain monitor rendering.
    ColorMonitorNtsc = 3,
    /// Color TV — NTSC signal-chain TV rendering (default, matches AppleWin).
    #[default]
    ColorTV         = 4,
    /// Monochrome TV (white phosphor, composite-TV bandwidth).
    MonoTV          = 5,
    /// Monochrome Amber phosphor.
    MonoAmber       = 6,
    /// Monochrome Green phosphor.
    MonoGreen       = 7,
    /// Monochrome White (pure white phosphor).
    MonoWhite       = 8,
}

impl VideoType {
    /// Return the RGB tint for monochrome modes.
    /// Returns `None` for colour modes (caller should use `monochrome_color`).
    pub fn mono_tint(self) -> Option<[u8; 3]> {
        match self {
            VideoType::MonoAmber  => Some([0xFF, 0x80, 0x00]),
            VideoType::MonoGreen  => Some([0x00, 0xC0, 0x00]),
            VideoType::MonoWhite | VideoType::MonoTV
                                  => Some([0xFF, 0xFF, 0xFF]),
            _                     => None,
        }
    }
}

// ── JoystickType ──────────────────────────────────────────────────────────────

/// Joystick emulation source, matching AppleWin's `JoyType_e`.
///
/// Stored as REGVALUE_JOYSTICK0_EMU_TYPE / REGVALUE_JOYSTICK1_EMU_TYPE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum JoystickType {
    /// Not connected (default).
    #[default]
    Disabled      = 0,
    /// PC joystick / gamepad #1.
    Joystick1     = 1,
    /// PC joystick / gamepad #2.
    Joystick2     = 2,
    /// Numeric keypad (2/4/6/8 + 0/5 fire).
    KeypadNumeric = 3,
    /// Arrow keys (+ space fire).
    KeypadArrows  = 4,
    /// Mouse cursor.
    Mouse         = 5,
}

pub fn joystick_type_name(j: JoystickType) -> &'static str {
    match j {
        JoystickType::Disabled      => "Not connected",
        JoystickType::Joystick1     => "Joystick 1",
        JoystickType::Joystick2     => "Joystick 2",
        JoystickType::KeypadNumeric => "Numeric keypad",
        JoystickType::KeypadArrows  => "Arrow keys",
        JoystickType::Mouse         => "Mouse cursor",
    }
}

pub const ALL_JOYSTICK_TYPES: &[JoystickType] = &[
    JoystickType::Disabled,
    JoystickType::Joystick1,
    JoystickType::Joystick2,
    JoystickType::KeypadNumeric,
    JoystickType::KeypadArrows,
    JoystickType::Mouse,
];

// ── Config struct ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    // ── Machine ───────────────────────────────────────────────────────────────
    pub machine_type: Apple2Model,
    pub cpu_type:     CpuType,

    // ── Slots (card type for each of the 8 slots, 0–7) ───────────────────────
    /// Card installed in each slot.  Serialised as 8-element TOML array of
    /// variant names (e.g. `["Empty","Empty","Empty","Empty","Empty","Empty","Disk2","Empty"]`).
    pub slot_cards: [CardType; 8],
    /// Card installed in the auxiliary slot (slot 8 / SLOT_AUX).
    /// Typically Extended80Col on //e, or RamWorksIII.
    pub aux_slot_card: CardType,

    // ── Video ─────────────────────────────────────────────────────────────────
    /// Video emulation type (matches AppleWin REGVALUE_VIDEO_MODE / VideoType_e).
    pub video_type:           VideoType,
    /// CRT half-scanline darkening (matches AppleWin VS_HALF_SCANLINES).
    pub scanlines:            bool,
    /// Vertical colour blending between adjacent scan-lines
    /// (matches AppleWin VS_COLOR_VERTICAL_BLEND).
    pub color_vertical_blend: bool,
    /// Custom monochrome colour as 0xRRGGBB (used when video_type = MonoCustom).
    /// Default matches AppleWin's default of light gray (0xC0C0C0).
    pub monochrome_color:     u32,
    /// Display refresh rate in Hz (50 or 60).
    pub video_refresh_hz:     u8,

    // ── Audio ─────────────────────────────────────────────────────────────────
    /// Master volume 0–100 (100 = full amplitude).
    /// AppleWin uses an inverted 0–59 scale (0=max, 59=silent); we use the
    /// more intuitive percentage form.
    pub master_volume: u8,

    // ── Speed ─────────────────────────────────────────────────────────────────
    /// CPU emulation speed, 0–40.  10 = SPEED_NORMAL (1.023 MHz).
    /// Matches AppleWin's REGVALUE_EMULATION_SPEED range.
    pub emulation_speed: u32,
    /// Run at 16× speed while the disk motor spins (fast boot).
    pub enhanced_disk_speed: bool,

    // ── Input / Joystick ──────────────────────────────────────────────────────
    /// Joystick 0 emulation source (matches REGVALUE_JOYSTICK0_EMU_TYPE).
    pub joystick0_type: JoystickType,
    /// Joystick 1 emulation source (matches REGVALUE_JOYSTICK1_EMU_TYPE).
    pub joystick1_type: JoystickType,
    /// Paddle X-axis trim –128 … +127 (REGVALUE_PDL_XTRIM).
    pub paddle_x_trim:  i8,
    /// Paddle Y-axis trim –128 … +127 (REGVALUE_PDL_YTRIM).
    pub paddle_y_trim:  i8,
    /// Allow joystick to use cursor keys for navigation (REGVALUE_CURSOR_CONTROL).
    pub joystick_cursor_control: bool,
    /// Self-centring behaviour (REGVALUE_CENTERING_CONTROL).
    pub joystick_self_centering: bool,
    /// Auto-fire on button 0 (REGVALUE_AUTOFIRE).
    pub joystick_autofire: bool,
    /// Swap joystick buttons 0 and 1 (REGVALUE_SWAP_BUTTONS_0_AND_1).
    pub joystick_swap_buttons: bool,
    /// Show a crosshair mouse cursor (REGVALUE_MOUSE_CROSSHAIR).
    pub mouse_crosshair: bool,
    /// Restrict mouse pointer to the emulator window (REGVALUE_MOUSE_RESTRICT_TO_WINDOW).
    pub mouse_restrict_to_window: bool,
    /// Map the Alt key to Open Apple (button 0).  Default true — matches
    /// the C++ AppleWin behaviour.
    pub alt_key_as_apple: bool,
    /// Memory Initialization Pattern (0–7).  Selects how RAM is filled on
    /// power-on reset.  Some copy-protected software requires specific patterns.
    pub memory_init_pattern: u8,

    // ── Custom ROM loading ───────────────────────────────────────────────────
    /// Path to a custom system ROM file (12K or 16K).  When set, this ROM is
    /// loaded instead of the embedded Apple IIe Enhanced ROM.
    pub custom_rom_path: Option<String>,
    /// Path to a custom F8 ROM (2K) that replaces only the $F800–$FFFF region.
    pub custom_f8_rom_path: Option<String>,

    // ── UI behaviour ──────────────────────────────────────────────────────────
    /// Show a confirmation dialog before any reset (matches AppleWin's
    /// `Confirm Reboot` registry value; default true).
    pub confirm_reboot: bool,
    /// Show Disk II activity LEDs in the status bar (REGVALUE_SHOW_DISKII_STATUS).
    /// Default true — always shown in windowed mode (matches our layout).
    pub show_disk_status: bool,
    /// Toggle ScrollLock to pause emulation (REGVALUE_SCROLLLOCK_TOGGLE).
    pub scrolllock_toggle: bool,

    // ── Window ────────────────────────────────────────────────────────────────
    /// Integer display scale factor (1=1×, 2=2×, …).
    /// Saved so the window opens at the same size on next launch.
    pub window_scale: u32,
    /// Last window X position (pixels from left of primary monitor).
    /// Not saved/restored when the window was maximized.
    pub window_x: Option<i32>,
    /// Last window Y position (pixels from top of primary monitor).
    /// Not saved/restored when the window was maximized.
    pub window_y: Option<i32>,
    /// Whether the window was maximized when last closed.
    /// Restored on next launch so the window reopens in the same state.
    pub window_maximized: bool,

    // ── Save state ────────────────────────────────────────────────────────────
    /// Automatically save a snapshot when the emulator exits and restore it
    /// on the next launch (matches `Save State On Exit`).
    pub save_state_on_exit: bool,
    /// Path of the save-state YAML file.  `None` → auto-generated beside the
    /// config file as `applewin-rs.aws.yaml`.
    pub save_state_filename: Option<String>,

    // ── Disk paths (persisted across sessions) ────────────────────────────────
    pub last_disk1:    Option<String>,
    pub last_disk2:    Option<String>,
    /// Starting directory for the floppy "Open disk" file dialog.
    pub last_disk_dir: Option<String>,
    /// Starting directory for the hard-disk image file dialog.
    pub last_hdd_dir:  Option<String>,
    /// Last hard-disk image in HDD drive 1 (REGVALUE_LAST_HARDDISK_1).
    pub last_hdd1:     Option<String>,
    /// Last hard-disk image in HDD drive 2 (REGVALUE_LAST_HARDDISK_2).
    pub last_hdd2:     Option<String>,

    // ── Recent file lists ─────────────────────────────────────────────────────
    /// Up to 10 most-recently-used floppy disk image paths.
    pub recent_disks: Vec<String>,
    /// Up to 5 most-recently-used HDD image paths.
    pub recent_hdds:  Vec<String>,

    // ── New-disk copy options ─────────────────────────────────────────────────
    /// Embed ProDOS 2.4.3 when creating a new ProDOS disk (mirrors C++ REGVALUE_PREF_NEW_DISK_COPY_PRODOS_SYS).
    pub new_disk_copy_prodos:     bool,
    /// Embed BASIC.SYSTEM 1.7 when creating a new ProDOS disk.
    pub new_disk_copy_basic:      bool,
    /// Embed BITSY.BOOT when creating a new ProDOS disk.
    pub new_disk_copy_bitsy_boot: bool,
    /// Embed QUIT.SYSTEM when creating a new ProDOS disk.
    pub new_disk_copy_bitsy_bye:  bool,

    // ── Per-card options ──────────────────────────────────────────────────────
    /// Mockingboard: enable SSI263 speech chips (default false).
    pub mockingboard_has_speech: bool,
    /// Phasor: true = Phasor native mode, false = Mockingboard compatible (default true).
    pub phasor_native_mode: bool,
    /// Saturn 128K: active RAM size in KB (16 / 32 / 64 / 128; default 128).
    pub saturn_ram_kb: u32,
    /// RamWorks III: RAM size in KB (64 / 128 / 256 / 512 / 1024 / 2048 / 4096 / 8192; default 1024).
    pub ramworks_ram_kb: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            machine_type:         Apple2Model::AppleIIeEnh,
            cpu_type:             CpuType::Cpu65C02,
            slot_cards:           default_slot_cards(),
            aux_slot_card:        CardType::Extended80Col,
            video_type:           VideoType::ColorTV,
            scanlines:            false,
            color_vertical_blend: false,
            monochrome_color:     0xC0C0C0, // AppleWin default: light gray
            video_refresh_hz:     60,
            master_volume:        100,
            emulation_speed:      10,   // SPEED_NORMAL
            enhanced_disk_speed:  true,
            joystick0_type:       JoystickType::Disabled,
            joystick1_type:       JoystickType::Disabled,
            paddle_x_trim:        0,
            paddle_y_trim:        0,
            joystick_cursor_control:  false,
            joystick_self_centering:  true,
            joystick_autofire:        false,
            joystick_swap_buttons:    false,
            mouse_crosshair:          false,
            mouse_restrict_to_window: false,
            alt_key_as_apple:         true,
            memory_init_pattern:      0,
            custom_rom_path:          None,
            custom_f8_rom_path:       None,
            confirm_reboot:       true,
            show_disk_status:     true,   // always show disk LEDs by default
            scrolllock_toggle:    false,
            window_scale:         2,
            window_x:             None,
            window_y:             None,
            window_maximized:     false,
            save_state_on_exit:   false,
            save_state_filename:  None,
            last_disk1:           None,
            last_disk2:           None,
            last_disk_dir:        None,
            last_hdd_dir:         None,
            last_hdd1:            None,
            last_hdd2:            None,
            recent_disks:         Vec::new(),
            recent_hdds:          Vec::new(),
            new_disk_copy_prodos:     true,
            new_disk_copy_basic:      true,
            new_disk_copy_bitsy_boot: true,
            new_disk_copy_bitsy_bye:  true,
            mockingboard_has_speech: false,
            phasor_native_mode:   true,
            saturn_ram_kb:        128,
            ramworks_ram_kb:      1024,
        }
    }
}

/// Default slot layout — Disk II in slot 6, everything else empty.
pub fn default_slot_cards() -> [CardType; 8] {
    [
        CardType::Empty, // slot 0
        CardType::Empty, // slot 1
        CardType::Empty, // slot 2
        CardType::Empty, // slot 3
        CardType::Empty, // slot 4
        CardType::Empty, // slot 5
        CardType::Disk2, // slot 6  ← standard Apple II configuration
        CardType::Empty, // slot 7
    ]
}

impl Config {
    /// Push a disk path to the front of the recent-disks list (max 10, deduped).
    pub fn add_recent_disk(&mut self, path: &str) {
        self.recent_disks.retain(|p| p != path);
        self.recent_disks.insert(0, path.to_string());
        self.recent_disks.truncate(10);
    }

    /// Push an HDD path to the front of the recent-HDDs list (max 5, deduped).
    pub fn add_recent_hdd(&mut self, path: &str) {
        self.recent_hdds.retain(|p| p != path);
        self.recent_hdds.insert(0, path.to_string());
        self.recent_hdds.truncate(5);
    }

    /// Return the index of the first slot that contains a Disk II card,
    /// or 6 as the fall-back default.
    pub fn disk2_slot(&self) -> usize {
        self.slot_cards
            .iter()
            .position(|&c| c == CardType::Disk2)
            .unwrap_or(6)
    }

    /// Return the path of the save-state file, generating a default beside the
    /// config file if none is explicitly configured.
    pub fn save_state_path(&self) -> Option<PathBuf> {
        if let Some(ref p) = self.save_state_filename {
            return Some(PathBuf::from(p));
        }
        Some(config_dir()?.join("applewin-rs.aws.yaml"))
    }

    /// CPU cycles to execute per display frame, accounting for emulation speed.
    ///
    /// At `emulation_speed = 10` (SPEED_NORMAL) and 60 Hz: 1_023_000 / 60 = 17_050 cycles.
    pub fn cycles_per_frame(&self) -> u64 {
        let speed = self.emulation_speed.max(1) as u64;
        speed * 1_023_000 / (10 * self.video_refresh_hz.max(1) as u64)
    }

    /// Return the monochrome tint RGB for the current video_type.
    /// For MonoCustom, reads `monochrome_color`; for colour modes returns None.
    pub fn mono_tint(&self) -> Option<[u8; 3]> {
        if self.video_type == VideoType::MonoCustom {
            let r = ((self.monochrome_color >> 16) & 0xFF) as u8;
            let g = ((self.monochrome_color >>  8) & 0xFF) as u8;
            let b = ( self.monochrome_color        & 0xFF) as u8;
            Some([r, g, b])
        } else {
            self.video_type.mono_tint()
        }
    }
}

// ── Load / save ───────────────────────────────────────────────────────────────

impl Config {
    /// Load from the platform config file. Returns `Default` on any error.
    pub fn load() -> Self {
        let Some(path) = config_path() else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&path) else { return Self::default() };
        match toml::from_str::<Config>(&text) {
            Ok(c)  => c,
            Err(e) => {
                eprintln!("Warning: config parse error ({e}), using defaults");
                Self::default()
            }
        }
    }

    /// Save to the platform config file. Silently ignores errors.
    pub fn save(&self) {
        let Some(dir) = config_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

// ── Platform path helpers ─────────────────────────────────────────────────────

fn config_path() -> Option<PathBuf> {
    Some(config_dir()?.join("config.toml"))
}

pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("applewin-rs"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("applewin-rs"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let home = std::env::var_os("HOME").unwrap_or_default();
                PathBuf::from(home).join(".config")
            });
        Some(base.join("applewin-rs"))
    }
}

// ── Display helpers used by the settings UI ───────────────────────────────────

pub fn model_name(m: Apple2Model) -> &'static str {
    match m {
        Apple2Model::AppleII      => "Apple II",
        Apple2Model::AppleIIPlus  => "Apple II+",
        Apple2Model::AppleIIe     => "Apple IIe",
        Apple2Model::AppleIIeEnh  => "Apple IIe Enhanced",
        Apple2Model::AppleIIc     => "Apple IIc",
        Apple2Model::AppleIIcPlus => "Apple IIc+",
        Apple2Model::AppleIIgs    => "Apple IIgs",
        Apple2Model::Clone        => "Clone",
    }
}

pub fn cpu_name(c: CpuType) -> &'static str {
    match c {
        CpuType::Cpu6502  => "6502",
        CpuType::Cpu65C02 => "65C02",
        CpuType::CpuZ80   => "Z80",
    }
}

pub fn video_type_name(v: VideoType) -> &'static str {
    match v {
        VideoType::MonoCustom       => "Monochrome (Custom)",
        VideoType::ColorIdealized   => "Color (Composite Idealized)",
        VideoType::ColorRGB         => "Color (RGB Card/Monitor)",
        VideoType::ColorMonitorNtsc => "Color Monitor NTSC",
        VideoType::ColorTV          => "Color TV",
        VideoType::MonoTV           => "Monochrome TV",
        VideoType::MonoAmber        => "Monochrome Amber",
        VideoType::MonoGreen        => "Monochrome Green",
        VideoType::MonoWhite        => "Monochrome White",
    }
}

/// Human-readable name for a card type (for the Slots tab).
pub fn card_name(c: CardType) -> &'static str {
    match c {
        CardType::Empty          => "Empty",
        CardType::Disk2          => "Disk II",
        CardType::Ssc            => "Super Serial Card",
        CardType::Mockingboard   => "Mockingboard",
        CardType::GenericPrinter => "Generic Printer",
        CardType::GenericHdd     => "Hard Disk",
        CardType::GenericClock   => "Clock",
        CardType::MouseInterface => "Mouse Interface",
        CardType::Z80            => "Z80 SoftCard",
        CardType::Phasor         => "Phasor",
        CardType::Echo           => "Echo+",
        CardType::Sam            => "SAM",
        CardType::Col80          => "80-Column (1K)",
        CardType::Extended80Col  => "80-Column Extended (64K)",
        CardType::RamWorksIII    => "RamWorks III",
        CardType::Uthernet       => "Uthernet",
        CardType::LanguageCard   => "Language Card",
        CardType::LanguageCardIIe=> "Language Card IIe",
        CardType::Saturn128K     => "Saturn 128K",
        CardType::FourPlay       => "4Play Joystick",
        CardType::SnesMax        => "SNES MAX",
        CardType::VidHD          => "VidHD",
        CardType::Uthernet2      => "Uthernet II",
        CardType::MegaAudio      => "MegaAudio",
        CardType::SdMusic        => "SD Music",
        CardType::BreakpointCard => "Breakpoint Card",
    }
}

/// Cards that are fully implemented in the Rust emulator and can be selected.
pub const IMPLEMENTED_CARDS: &[CardType] = &[
    CardType::Empty,
    CardType::Disk2,
    CardType::GenericHdd,
    CardType::Mockingboard,
    CardType::MouseInterface,
    CardType::Ssc,
    // Previously implemented in last session:
    CardType::Phasor,
    CardType::Col80,
    CardType::Extended80Col,
    CardType::Sam,
    CardType::GenericClock,
    CardType::FourPlay,
    CardType::SnesMax,
    CardType::Saturn128K,
    CardType::RamWorksIII,
    // New in this session:
    CardType::GenericPrinter,
    CardType::VidHD,
    CardType::Z80,
    CardType::Uthernet,
    CardType::Uthernet2,
    CardType::LanguageCard,
    CardType::MegaAudio,
    CardType::SdMusic,
];

/// All video types in menu order (matches AppleWin property sheet).
pub const ALL_VIDEO_TYPES: &[VideoType] = &[
    VideoType::ColorTV,
    VideoType::ColorIdealized,
    VideoType::ColorMonitorNtsc,
    VideoType::ColorRGB,
    VideoType::MonoTV,
    VideoType::MonoAmber,
    VideoType::MonoGreen,
    VideoType::MonoWhite,
    VideoType::MonoCustom,
];
