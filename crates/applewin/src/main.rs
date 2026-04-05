use apple2_core::{
    emulator::Emulator,
    model::{Apple2Model, CpuType},
};

/// Apple IIe Enhanced 16KB ROM embedded at compile time.
static APPLE2E_ROM: &[u8] = include_bytes!("../roms/apple2e_enhanced.rom");

#[cfg(feature = "gui")]
mod config;

fn make_emulator(machine: Apple2Model, cpu: CpuType) -> Emulator {
    let rom = APPLE2E_ROM.to_vec();
    Emulator::new(rom, machine, cpu)
    // Card insertion is handled by apply_slot_cards() in the gui module.
}

fn main() {
    println!("AppleWin-rs v{}", env!("CARGO_PKG_VERSION"));

    #[cfg(feature = "gui")]
    {
        let cfg = config::Config::load();
        let emu = make_emulator(cfg.machine_type, cfg.cpu_type);
        // Mode stays as AppMode::Logo — the GUI will show the logo screen
        // and switch to Running on the first key press.
        println!(
            "Emulator initialised — model={:?}  PC=${:04X}",
            emu.model, emu.cpu.pc
        );
        gui::run(emu, cfg);
    }

    #[cfg(not(feature = "gui"))]
    {
        let mut emu = make_emulator(Apple2Model::AppleIIeEnh, CpuType::Cpu65C02);
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
        println!("Headless — executed {} cycles, PC=${:04X}", executed, emu.cpu.pc);
    }
}

// ── GUI (eframe 0.23 + egui 0.23) ────────────────────────────────────────────

#[cfg(feature = "gui")]
mod gui {
    use apple2_core::emulator::Emulator;
    use apple2_video::{
        framebuffer::Framebuffer,
        ntsc::{CharRom, NtscRenderer},
    };
    use eframe::egui::{
        self, Align, Color32, ColorImage, FontId, Key, Layout, Pos2, Rect, RichText,
        Sense, Stroke, TextureOptions, Vec2,
    };
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use crate::config::{
        card_name, Config, cpu_name, joystick_type_name, model_name, video_type_name,
        VideoType, ALL_JOYSTICK_TYPES, ALL_VIDEO_TYPES, IMPLEMENTED_CARDS,
    };
    use apple2_core::{card::CardType, model::{Apple2Model, CpuType}};

    // ── Dimensions ───────────────────────────────────────────────────────────

    const SCREEN_W: usize = 560;
    const SCREEN_H: usize = 384;
    /// 3D bevel around the Apple II screen (2 layers × 2 px each)
    const BEVEL: f32 = 4.0;
    /// Width of the right-side button strip
    const BTN_PANEL_W: f32 = 55.0;

    // ── Windows 9x-style palette for chrome ──────────────────────────────────

    const WIN_FACE:    Color32 = Color32::from_rgb(212, 208, 200);
    const WIN_LIGHT:   Color32 = Color32::from_rgb(255, 255, 255);
    const WIN_HILIGHT: Color32 = Color32::from_rgb(212, 208, 200);
    const WIN_SHADOW:  Color32 = Color32::from_rgb(128, 128, 128);
    const WIN_DSHADOW: Color32 = Color32::from_rgb(64,  64,  64);

    // ── Embedded toolbar BMP icons ────────────────────────────────────────────

    static BMP_HELP:  &[u8] = include_bytes!("../icons/HELP.BMP");
    static BMP_RUN:   &[u8] = include_bytes!("../icons/RUN.BMP");
    static BMP_D1:    &[u8] = include_bytes!("../icons/DRIVE1.BMP");
    static BMP_D2:    &[u8] = include_bytes!("../icons/DRIVE2.BMP");
    static BMP_SWAP:  &[u8] = include_bytes!("../icons/DriveSwap.bmp");
    static BMP_FULL:  &[u8] = include_bytes!("../icons/FULLSCR.BMP");
    static BMP_DEBUG: &[u8] = include_bytes!("../icons/DEBUG.BMP");
    static BMP_SETUP: &[u8] = include_bytes!("../icons/SETUP.BMP");
    static BMP_LOGO:  &[u8] = include_bytes!("../icons/ApplewinLogo.bmp");

    /// Decode a Windows indexed-colour BMP (4bpp or 8bpp) to RGBA8888 pixels.
    ///
    /// Cyan (0, 255, 255) is treated as fully transparent (chroma-key).
    fn decode_bmp_rgba(data: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
        if data.len() < 54 || &data[0..2] != b"BM" { return None; }
        let pixel_offset = u32::from_le_bytes(data[10..14].try_into().ok()?) as usize;
        let w = i32::from_le_bytes(data[18..22].try_into().ok()?) as usize;
        let h_raw = i32::from_le_bytes(data[22..26].try_into().ok()?);
        let h = h_raw.unsigned_abs() as usize;
        let bpp = u16::from_le_bytes(data[28..30].try_into().ok()?);
        let colors_used = u32::from_le_bytes(data[46..50].try_into().ok()?) as usize;

        let num_colors: usize = match bpp {
            4 => if colors_used > 0 { colors_used } else { 16 },
            8 => if colors_used > 0 { colors_used } else { 256 },
            _ => return None,
        };

        // Build palette: BMP stores RGBQUAD as (blue, green, red, reserved)
        let pal_start = 54usize;
        if data.len() < pal_start + num_colors * 4 { return None; }
        let mut palette = Vec::with_capacity(num_colors);
        for i in 0..num_colors {
            let b = data[pal_start + i * 4];
            let g = data[pal_start + i * 4 + 1];
            let r = data[pal_start + i * 4 + 2];
            palette.push((r, g, b));
        }

        let flip = h_raw > 0; // positive height = bottom-to-top storage
        let row_stride = match bpp {
            4 => (w * 4).div_ceil(32) * 4,
            8 => w.div_ceil(4) * 4,
            _ => return None,
        };

        let mut rgba = vec![0u8; w * h * 4];
        for row in 0..h {
            let src_row = if flip { h - 1 - row } else { row };
            let src_off = pixel_offset + src_row * row_stride;
            for x in 0..w {
                let idx: usize = match bpp {
                    4 => {
                        let byte = *data.get(src_off + x / 2)?;
                        if x % 2 == 0 { (byte >> 4) as usize } else { (byte & 0xF) as usize }
                    }
                    8 => *data.get(src_off + x)? as usize,
                    _ => return None,
                };
                let (r, g, b) = *palette.get(idx)?;
                // Cyan (0, 255, 255) is the chroma-key colour used by Win32 toolbar
                let a = if r == 0 && g == 255 && b == 255 { 0u8 } else { 255u8 };
                let dst = (row * w + x) * 4;
                rgba[dst]     = r;
                rgba[dst + 1] = g;
                rgba[dst + 2] = b;
                rgba[dst + 3] = a;
            }
        }
        Some((w, h, rgba))
    }

    /// Decode a 24bpp Windows BMP to RGBA8888 pixels.
    fn decode_bmp24_rgba(data: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
        if data.len() < 54 || &data[0..2] != b"BM" { return None; }
        let pixel_offset = u32::from_le_bytes(data[10..14].try_into().ok()?) as usize;
        let w = i32::from_le_bytes(data[18..22].try_into().ok()?) as usize;
        let h_raw = i32::from_le_bytes(data[22..26].try_into().ok()?);
        let h = h_raw.unsigned_abs() as usize;
        let bpp = u16::from_le_bytes(data[28..30].try_into().ok()?);
        if bpp != 24 { return None; }
        let flip = h_raw > 0;
        let row_stride = (w * 3).div_ceil(4) * 4;
        let mut rgba = vec![0u8; w * h * 4];
        for row in 0..h {
            let src_row = if flip { h - 1 - row } else { row };
            let src_off = pixel_offset + src_row * row_stride;
            for x in 0..w {
                let b = *data.get(src_off + x * 3)?;
                let g = *data.get(src_off + x * 3 + 1)?;
                let r = *data.get(src_off + x * 3 + 2)?;
                let dst = (row * w + x) * 4;
                rgba[dst]     = r;
                rgba[dst + 1] = g;
                rgba[dst + 2] = b;
                rgba[dst + 3] = 255;
            }
        }
        Some((w, h, rgba))
    }

    // ── Audio ─────────────────────────────────────────────────────────────────

    type AudioBuf = Arc<Mutex<VecDeque<f32>>>;

    /// Apple II CPU clock (Hz) — NTSC.
    const CPU_HZ: f64 = 1_023_000.0;
    /// Maximum audio ring-buffer size in samples (2 seconds at 48 kHz).
    const AUDIO_BUF_MAX: usize = 96_000;

    /// Initialise cpal audio output.  Returns `(sample_rate, shared_buf, stream)`.
    /// The stream must be kept alive for the duration of the program.
    fn init_audio() -> Option<(u32, AudioBuf, cpal::Stream)> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host   = cpal::default_host();
        let device = host.default_output_device()?;
        let config = device.default_output_config().ok()?;
        let sr     = config.sample_rate().0;
        let ch     = config.channels() as usize;

        let buf: AudioBuf = Arc::new(Mutex::new(VecDeque::with_capacity(8192)));
        let buf2 = buf.clone();

        let err_fn = |e: cpal::StreamError| eprintln!("audio stream error: {e}");

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_output_stream(
                    &config.into(),
                    move |data: &mut [f32], _| {
                        let mut q = buf2.lock().unwrap();
                        for frame in data.chunks_mut(ch) {
                            let s = q.pop_front().unwrap_or(0.0);
                            for c in frame.iter_mut() { *c = s; }
                        }
                    },
                    err_fn,
                    None,
                ).ok()?
            }
            cpal::SampleFormat::I16 => {
                device.build_output_stream(
                    &config.into(),
                    move |data: &mut [i16], _| {
                        let mut q = buf2.lock().unwrap();
                        for frame in data.chunks_mut(ch) {
                            let s = q.pop_front().unwrap_or(0.0);
                            let v = (s * i16::MAX as f32) as i16;
                            for c in frame.iter_mut() { *c = v; }
                        }
                    },
                    err_fn,
                    None,
                ).ok()?
            }
            cpal::SampleFormat::U16 => {
                device.build_output_stream(
                    &config.into(),
                    move |data: &mut [u16], _| {
                        let mut q = buf2.lock().unwrap();
                        for frame in data.chunks_mut(ch) {
                            let s = q.pop_front().unwrap_or(0.0);
                            let v = ((s + 1.0) * 0.5 * u16::MAX as f32) as u16;
                            for c in frame.iter_mut() { *c = v; }
                        }
                    },
                    err_fn,
                    None,
                ).ok()?
            }
            _ => return None,
        };

        stream.play().ok()?;
        Some((sr, buf, stream))
    }

    /// All 8 toolbar icon textures, pre-loaded at startup.
    struct Icons {
        help:  Option<egui::TextureHandle>,
        run:   Option<egui::TextureHandle>,
        d1:    Option<egui::TextureHandle>,
        d2:    Option<egui::TextureHandle>,
        swap:  Option<egui::TextureHandle>,
        full:  Option<egui::TextureHandle>,
        debug: Option<egui::TextureHandle>,
        setup: Option<egui::TextureHandle>,
    }

    impl Icons {
        fn load(ctx: &egui::Context) -> Self {
            let opts = TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification:  egui::TextureFilter::Nearest,
            };
            let load = |name: &str, raw: &[u8]| -> Option<egui::TextureHandle> {
                let (w, h, rgba) = decode_bmp_rgba(raw)?;
                let img = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                Some(ctx.load_texture(name, img, opts))
            };
            Self {
                help:  load("icon_help",  BMP_HELP),
                run:   load("icon_run",   BMP_RUN),
                d1:    load("icon_d1",    BMP_D1),
                d2:    load("icon_d2",    BMP_D2),
                swap:  load("icon_swap",  BMP_SWAP),
                full:  load("icon_full",  BMP_FULL),
                debug: load("icon_debug", BMP_DEBUG),
                setup: load("icon_setup", BMP_SETUP),
            }
        }
    }

    // ── App state ─────────────────────────────────────────────────────────────

    struct EmulatorApp {
        emu:              Emulator,
        renderer:         NtscRenderer,
        fb:               Framebuffer,
        pixel_buf:        Vec<u8>,
        texture:          Option<egui::TextureHandle>,
        logo_texture:     Option<egui::TextureHandle>,
        icons:            Option<Icons>,
        frame_no:         u32,
        disk1:            Option<PathBuf>,
        disk2:            Option<PathBuf>,
        fullscreen:       bool,
        show_about:       bool,
        // Configuration
        config:           Config,
        show_settings:    bool,
        pending_config:   Config,
        settings_tab:     usize,
        /// Which slot the Disk II card is currently installed in (derived from config).
        disk_slot:        usize,
        /// Pending reset type: Some(true)=hard-reset, Some(false)=soft-reset,
        /// None=no pending.  Set when confirm_reboot is true.
        pending_reset:    Option<bool>,
        // Audio
        audio_buf:         Option<AudioBuf>,
        _audio_stream:     Option<cpal::Stream>,
        speaker_state:     bool,
        last_audio_cycle:  u64,
        audio_sample_rate: u32,
        /// Fractional CPU cycles that didn't make a full sample last frame.
        spkr_cycle_rem:    f64,
        /// DC-filter counter (matches Windows `g_uDCFilterState`).
        /// Reset to 32768+10000 on every $C030 toggle; linearly fades to 0.
        dc_filter_ctr:     u32,
        /// Wall-clock timestamp of the previous update() call.
        /// Used to execute exactly the right number of CPU cycles regardless of
        /// how often egui calls update() (window resize can cause burst repaints).
        last_frame_time:   std::time::Instant,
        /// Characters queued for paste injection into the Apple II keyboard.
        paste_buf:         std::collections::VecDeque<u8>,
        // Debugger
        show_debugger:     bool,
        debugger:          apple2_debugger::DebuggerState,
        debugger_cmd_input: String,
        #[allow(dead_code)]
        debugger_bp_input: String,
        #[allow(dead_code)]
        debugger_mem_input: String,
        #[allow(dead_code)]
        debugger_reg_input: String,
        #[allow(dead_code)]
        debugger_tab:      usize,
        // Per-slot options popup open flags (one per slot, 0..8)
        slot_options_open: [bool; 8],
    }

    impl EmulatorApp {
        fn new(mut emu: Emulator, config: Config) -> Self {
            // Install cards according to slot configuration
            apply_slot_cards(&mut emu, &config);

            // Build CharRom from the embedded Apple IIe video ROM
            let font_data = build_font_from_rom(VIDEO_ROM);
            let mut renderer = NtscRenderer::new(CharRom::new(font_data), config.scanlines);
            renderer.mono_tint            = config.mono_tint();
            renderer.color_vertical_blend = config.color_vertical_blend;

            // Initialise audio output (best-effort; silent if unavailable)
            let (audio_buf, _audio_stream, audio_sample_rate) =
                match init_audio() {
                    Some((sr, buf, stream)) => (Some(buf), Some(stream), sr),
                    None => {
                        eprintln!("Warning: audio output unavailable");
                        (None, None, 44100)
                    }
                };

            // Derive disk slot from config before auto-loading disks
            let disk_slot = config.disk2_slot();

            // Auto-load last disks from config
            let mut disk1: Option<PathBuf> = None;
            let mut disk2: Option<PathBuf> = None;
            if let Some(ref p) = config.last_disk1 {
                let path = PathBuf::from(p);
                if let Ok(data) = std::fs::read(&path) {
                    let ext = path.extension()
                        .and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    emu.bus.load_disk(disk_slot, 0, &data, &ext);
                    emu.bus.set_disk_path(disk_slot, 0, path.clone());
                    disk1 = Some(path);
                }
            }
            if let Some(ref p) = config.last_disk2 {
                let path = PathBuf::from(p);
                if let Ok(data) = std::fs::read(&path) {
                    let ext = path.extension()
                        .and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                    emu.bus.load_disk(disk_slot, 1, &data, &ext);
                    emu.bus.set_disk_path(disk_slot, 1, path.clone());
                    disk2 = Some(path);
                }
            }

            // Restore snapshot from previous session if enabled.
            // If a snapshot is restored we skip the logo screen and go straight
            // to Running (the user already saw the logo last session).
            if config.save_state_on_exit
                && let Some(path) = config.save_state_path()
                && let Ok(yaml) = std::fs::read_to_string(&path)
                && let Ok(snap) = serde_yaml::from_str::<
                    apple2_core::emulator::EmulatorSnapshot,
                >(&yaml)
            {
                emu.restore_snapshot(&snap);
                emu.mode = apple2_core::emulator::AppMode::Running;
            }

            let initial_cycle = emu.cpu.cycles;
            let pending_config = config.clone();

            Self {
                emu,
                renderer,
                fb:               Framebuffer::new(),
                pixel_buf:        vec![0u8; SCREEN_W * SCREEN_H * 4],
                texture:          None,
                logo_texture:     None,
                icons:            None,
                frame_no:         0,
                disk1,
                disk2,
                fullscreen:       false,
                show_about:       false,
                config,
                show_settings:    false,
                pending_config,
                settings_tab:     0,
                disk_slot,
                pending_reset:    None,
                audio_buf,
                _audio_stream,
                speaker_state:     false,
                last_audio_cycle:  initial_cycle,
                audio_sample_rate,
                spkr_cycle_rem:    0.0,
                dc_filter_ctr:     0,
                last_frame_time:   std::time::Instant::now(),
                paste_buf:         std::collections::VecDeque::new(),
                show_debugger:     false,
                debugger:          {
                    let mut d = apple2_debugger::DebuggerState::new();
                    d.load_apple2_symbols();
                    d
                },
                debugger_cmd_input: String::new(),
                debugger_bp_input: String::new(),
                debugger_mem_input: String::new(),
                debugger_reg_input: String::new(),
                debugger_tab:      0,
                slot_options_open: [false; 8],
            }
        }

        /// Reset the emulator and clear all audio state so no stale signal leaks through.
        fn reset(&mut self, power_cycle: bool) {
            self.emu.reset(power_cycle);
            // Silence the speaker: discard any pending toggles and reset the DC
            // filter so we don't output a fading ±0.5 hiss after the reset.
            self.speaker_state    = false;
            self.dc_filter_ctr    = 0;
            self.spkr_cycle_rem   = 0.0;
            self.last_audio_cycle = self.emu.cpu.cycles;
            self.last_frame_time  = std::time::Instant::now();
        }

        fn reload_disk(emu: &mut Emulator, slot: usize, drive: usize, path: &Option<PathBuf>) {
            if let Some(p) = path {
                if let Ok(data) = std::fs::read(p) {
                    let ext = p.extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    emu.bus.load_disk(slot, drive, &data, &ext);
                    emu.bus.set_disk_path(slot, drive, p.clone());
                }
            } else {
                emu.bus.eject_disk(slot, drive);
            }
        }

        fn disk_display_name(path: &Option<PathBuf>) -> &str {
            path.as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("(empty)")
        }

        /// Render the Apple II screen into `self.pixel_buf` via `NtscRenderer`.
        fn render_apple2(&mut self) {
            self.renderer.render(
                &self.emu.bus.main_ram,
                &self.emu.bus.aux_ram,
                self.emu.bus.mode,
                self.frame_no,
                &mut self.fb,
            );
            self.pixel_buf.copy_from_slice(self.fb.pixels_as_bytes());
        }

        /// Render the debugger display into the framebuffer, replacing the
        /// Apple II screen — matching the original AppleWin behaviour.
        fn render_debugger(&mut self) {
            use apple2_debugger::display::{self, CpuSnapshot};
            let cpu = CpuSnapshot {
                pc: self.emu.cpu.pc,
                a:  self.emu.cpu.a,
                x:  self.emu.cpu.x,
                y:  self.emu.cpu.y,
                sp: self.emu.cpu.sp,
                flags: self.emu.cpu.flags.bits(),
                cycles: self.emu.cpu.cycles,
            };
            let mode_bits = self.emu.bus.mode.bits();
            let cmd_input = self.debugger_cmd_input.clone();
            display::render(
                self.fb.pixels_mut(),
                &self.debugger,
                &cpu,
                mode_bits,
                &cmd_input,
                |a| self.emu.bus.read_raw(a),
            );
            self.pixel_buf.copy_from_slice(self.fb.pixels_as_bytes());
        }
    }

    // ── eframe App impl ───────────────────────────────────────────────────────

    impl eframe::App for EmulatorApp {
        fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
            self.frame_no = self.frame_no.wrapping_add(1);

            // Load icons and logo texture on first frame
            if self.icons.is_none() {
                self.icons = Some(Icons::load(ctx));
            }
            if self.logo_texture.is_none()
                && let Some((w, h, rgba)) = decode_bmp24_rgba(BMP_LOGO)
            {
                let img = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                self.logo_texture = Some(ctx.load_texture("logo", img, TextureOptions {
                    magnification: egui::TextureFilter::Linear,
                    minification:  egui::TextureFilter::Linear,
                }));
            }

            let in_logo_mode = self.emu.mode == apple2_core::emulator::AppMode::Logo;

            // ── Run emulator for elapsed wall-clock time (not during logo) ───
            //
            // We execute cycles proportional to real time elapsed since the last
            // update() call rather than a fixed "one frame worth" of cycles.
            // This decouples emulator speed from repaint frequency: window resize,
            // settings dialogs, and OS events all cause extra egui repaints, which
            // would otherwise run the emulator at 2-3× speed and corrupt game state.
            //
            // Cap at 100 ms (≈6 frames) so that minimising the window or attaching
            // a debugger doesn't cause a huge burst of catch-up execution.
            // Capture wall-clock time once for the whole frame — avoids multiple
            // system calls across the logo/normal/full-speed branches.
            let frame_now = std::time::Instant::now();

            if !in_logo_mode {
                let elapsed_secs = frame_now
                    .duration_since(self.last_frame_time)
                    .as_secs_f64()
                    .min(0.1); // 100 ms cap
                self.last_frame_time = frame_now;

                // Skip execution entirely when the debugger has paused the CPU
                let debugger_paused = self.debugger.active;

                // CPU clock rate: emulation_speed * 102_300 Hz (1× = 1.023 MHz)
                let base_hz = self.config.emulation_speed.max(1) as f64 * 102_300.0;

                if debugger_paused {
                    // Debugger is paused — do not execute any cycles.
                    // Handle trace mode: if trace_remaining > 0, step and record
                    if self.debugger.trace_remaining > 0 {
                        use apple2_debugger::disasm::{disassemble_one, format_instruction};
                        use apple2_debugger::trace::TraceEntry;
                        let instr = disassemble_one(self.emu.cpu.pc, |a| self.emu.bus.read_raw(a));
                        let text = format_instruction(&instr);
                        self.debugger.trace.push(TraceEntry {
                            pc: self.emu.cpu.pc,
                            opcode: instr.opcode,
                            a: self.emu.cpu.a,
                            x: self.emu.cpu.x,
                            y: self.emu.cpu.y,
                            sp: self.emu.cpu.sp,
                            flags: self.emu.cpu.flags.bits(),
                            cycles: self.emu.cpu.cycles,
                            text: text.clone(),
                        });
                        self.debugger.print(text);
                        self.emu.step();
                        self.debugger.trace_remaining -= 1;
                        if self.debugger.trace_remaining == 0 {
                            self.debugger.activate(apple2_debugger::state::StopReason::TraceComplete);
                            self.debugger.print("Trace complete.");
                        }
                    }
                } else if self.config.enhanced_disk_speed && self.emu.bus.disk_motor_on() {
                    // Full-speed mode — matches AppleWin's g_bFullSpeed behaviour
                    const BUDGET: std::time::Duration = std::time::Duration::from_millis(100);
                    const BATCH: u64 = 100_000;
                    let full_start = std::time::Instant::now();
                    while self.emu.bus.disk_motor_on() && full_start.elapsed() < BUDGET {
                        self.emu.execute(BATCH);
                    }
                    self.last_frame_time = std::time::Instant::now();
                    self.emu.bus.speaker_toggles.clear();
                    self.last_audio_cycle = self.emu.cpu.cycles;
                    self.dc_filter_ctr    = 0;
                    self.speaker_state    = false;
                    self.spkr_cycle_rem   = 0.0;
                    if let Some(buf) = &self.audio_buf {
                        buf.lock().unwrap().clear();
                    }
                } else {
                    let cycles = (elapsed_secs * base_hz) as u64;
                    if cycles > 0 {
                        // Use callback-based execution to check breakpoints
                        let bps = &self.debugger.breakpoints;
                        let step_over = self.debugger.step_over_target;
                        let step_out_sp = self.debugger.step_out_sp;
                        let has_breakpoints = !bps.is_empty()
                            || step_over.is_some()
                            || step_out_sp.is_some();

                        if has_breakpoints {
                            let breakpoints = &self.debugger.breakpoints;
                            let mut hit_bp: Option<u16> = None;
                            let mut hit_step_over = false;
                            let _hit_step_out = false;

                            self.emu.execute_with_callback(cycles, |pc| {
                                // Check step-over target
                                if let Some(target) = step_over {
                                    if pc == target {
                                        hit_step_over = true;
                                        return false;
                                    }
                                }
                                // Check step-out (RTS detection)
                                // We can't easily check SP inside callback,
                                // so step-out is handled differently below
                                // Check PC breakpoints
                                if breakpoints.check_opcode(pc) {
                                    hit_bp = Some(pc);
                                    return false;
                                }
                                true
                            });

                            if let Some(addr) = hit_bp {
                                self.debugger.activate(
                                    apple2_debugger::state::StopReason::Breakpoint(addr),
                                );
                                self.debugger.print(format!("Breakpoint hit at ${addr:04X}"));
                                self.debugger.cursor = addr;
                                self.show_debugger = true;
                            }
                            if hit_step_over {
                                self.debugger.step_over_target = None;
                                self.debugger.activate(
                                    apple2_debugger::state::StopReason::StepOver,
                                );
                            }
                        } else {
                            self.emu.execute(cycles);
                        }
                    }
                }
            } else {
                // Keep last_frame_time current so we don't burst when logo exits.
                self.last_frame_time = frame_now;
            }

            // ── Speaker audio synthesis ───────────────────────────────────────
            // Mirrors AppleWin's UpdateSpkr() / DCFilter() logic from Speaker.cpp.
            {
                let end_cycle  = self.emu.cpu.cycles;
                let start_cycle = self.last_audio_cycle;
                self.last_audio_cycle = end_cycle;

                let toggles = std::mem::take(&mut self.emu.bus.speaker_toggles);

                if let Some(buf) = &self.audio_buf {
                    let sr = self.audio_sample_rate as f64;
                    // Cycles per audio sample (truncated to integer, matching the C++
                    // "Use integer value: Better for MJ Mahon's RT.SYNTH" comment).
                    let clks_per_sample =
                        (CPU_HZ / sr).floor().max(1.0);

                    // Total cycles this frame including any fractional carry-over.
                    let delta = end_cycle.saturating_sub(start_cycle) as f64
                        + self.spkr_cycle_rem;
                    let n_samples = (delta / clks_per_sample) as usize;
                    self.spkr_cycle_rem = delta - n_samples as f64 * clks_per_sample;

                    if n_samples > 0 {
                        let cycles_per_sample = delta / n_samples as f64;
                        let mut toggle_idx = 0usize;
                        // Hoist volume scale out of the per-sample loop.
                        let volume_scale = self.config.master_volume as f32 / 100.0;
                        let mut locked = buf.lock().unwrap();

                        for i in 0..n_samples {
                            let sample_end =
                                start_cycle as f64 + (i + 1) as f64 * cycles_per_sample;

                            // Apply every toggle whose cycle falls within this sample.
                            // On each toggle: reset the DC filter (matches ResetDCFilter()).
                            while toggle_idx < toggles.len()
                                && (toggles[toggle_idx] as f64) <= sample_end
                            {
                                self.speaker_state = !self.speaker_state;
                                self.dc_filter_ctr = 32_768 + 10_000;
                                toggle_idx += 1;
                            }

                            // Raw square wave ±0.5
                            let raw = if self.speaker_state { 0.5f32 } else { -0.5f32 };

                            // DC filter: mirrors DCFilter() in Speaker.cpp.
                            // Passes full amplitude for ~250 ms after last toggle,
                            // then linearly fades to zero over ~744 ms.
                            let out = if self.dc_filter_ctr == 0 {
                                0.0f32
                            } else if self.dc_filter_ctr >= 32_768 {
                                self.dc_filter_ctr -= 1;
                                raw
                            } else {
                                let gain = self.dc_filter_ctr as f32 / 32_768.0;
                                self.dc_filter_ctr -= 1;
                                raw * gain
                            };

                            if locked.len() < AUDIO_BUF_MAX {
                                locked.push_back(out * volume_scale);
                            }
                        }
                    }
                }
            }

            // ── Mockingboard audio ────────────────────────────────────────────
            // Drain audio from any Mockingboard cards and mix into the ring buffer.
            // Collect all card samples first, then lock the ring buffer once for
            // all slots instead of once per slot.
            if self.audio_buf.is_some() {
                use apple2_core::card::CardType;
                let frame_cycles  = self.config.cycles_per_frame();
                let sr            = self.audio_sample_rate;
                let volume_scale  = self.config.master_volume as f32 / 100.0;

                // Gather samples from every Mockingboard/Phasor slot before locking.
                // Pre-allocate: ~735 samples/frame at 44100 Hz, 2 cards max.
                let mut mb_samples: Vec<f32> = Vec::with_capacity(2048);
                for slot in 0..apple2_core::card::NUM_SLOTS {
                    if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                        && (card.card_type() == CardType::Mockingboard
                            || card.card_type() == CardType::Phasor
                            || card.card_type() == CardType::Sam)
                    {
                        card.fill_audio(&mut mb_samples, frame_cycles, sr);
                    }
                }

                // Single lock acquisition for all collected samples.
                if !mb_samples.is_empty()
                    && let Some(buf) = &self.audio_buf {
                    let mut locked = buf.lock().unwrap();
                    for s in mb_samples {
                        if locked.len() < AUDIO_BUF_MAX {
                            locked.push_back(s * volume_scale);
                        }
                    }
                }
            }

            // ── Collect input events ──────────────────────────────────────────
            let mut key_queue:      Vec<u8>         = Vec::new();
            let mut do_reset:       bool             = false;
            let mut do_hard_reset:  bool             = false;
            let mut do_quit:        bool             = false;
            let mut any_key:        bool             = false;
            let mut paste_text:     Option<String>   = None;
            let mut take_screenshot: bool            = false;
            let mut video_shortcut: Option<VideoType> = None;

            // Only process Event::Key with repeat:false — this fires exactly once
            // per physical key-down, never for OS auto-repeat.  Event::Text is
            // intentionally ignored because it fires on every repeat frame,
            // causing the Apple II to see the same letter dozens of times/second.
            ctx.input(|i| {
                for event in &i.events {
                    // Paste event — filled by Ctrl+V or OS paste gesture.
                    if let egui::Event::Paste(text) = event {
                        paste_text = Some(text.clone());
                        continue;
                    }
                    if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event {
                        let ctrl  = modifiers.ctrl || modifiers.command;
                        let shift = modifiers.shift;
                        match key {
                            Key::F1                => do_hard_reset = true,
                            Key::F10 if self.config.scrolllock_toggle => {
                                self.debugger.active = !self.debugger.active;
                            }
                            Key::F12               => take_screenshot = true,
                            Key::Escape if ctrl    => do_quit = true,
                            Key::F2 if ctrl        => do_reset = true,
                            // Ctrl+1..5 — video mode shortcuts
                            Key::Num1 if ctrl => { video_shortcut = Some(VideoType::ColorTV); }
                            Key::Num2 if ctrl => { video_shortcut = Some(VideoType::ColorIdealized); }
                            Key::Num3 if ctrl => { video_shortcut = Some(VideoType::ColorRGB); }
                            Key::Num4 if ctrl => { video_shortcut = Some(VideoType::MonoWhite); }
                            Key::Num5 if ctrl => { video_shortcut = Some(VideoType::MonoGreen); }
                            Key::Enter             => { any_key = true; key_queue.push(0x0D); }
                            Key::Backspace         => { any_key = true; key_queue.push(0x7F); }
                            Key::Escape            => { any_key = true; key_queue.push(0x1B); }
                            Key::Tab               => { any_key = true; key_queue.push(0x09); }
                            Key::ArrowLeft         => { any_key = true; key_queue.push(0x08); }
                            Key::ArrowRight        => { any_key = true; key_queue.push(0x15); }
                            Key::ArrowUp           => { any_key = true; key_queue.push(0x0B); }
                            Key::ArrowDown         => { any_key = true; key_queue.push(0x0A); }
                            key if ctrl => {
                                // Ctrl+letter → Apple II control character (0x01–0x1A).
                                // Ctrl+V is intercepted for host paste (Event::Paste above).
                                let c: Option<u8> = match key {
                                    Key::A => Some(0x01), Key::B => Some(0x02),
                                    Key::C => Some(0x03), Key::D => Some(0x04),
                                    Key::E => Some(0x05), Key::F => Some(0x06),
                                    Key::G => Some(0x07), Key::H => Some(0x08),
                                    Key::I => Some(0x09), Key::J => Some(0x0A),
                                    Key::K => Some(0x0B), Key::L => Some(0x0C),
                                    Key::M => Some(0x0D), Key::N => Some(0x0E),
                                    Key::O => Some(0x0F), Key::P => Some(0x10),
                                    Key::Q => Some(0x11), Key::R => Some(0x12),
                                    Key::S => Some(0x13), Key::T => Some(0x14),
                                    Key::U => Some(0x15), Key::V => None, // paste — see Event::Paste
                                    Key::W => Some(0x17), Key::X => Some(0x18),
                                    Key::Y => Some(0x19), Key::Z => Some(0x1A),
                                    _ => None,
                                };
                                if let Some(c) = c { any_key = true; key_queue.push(c); }
                            }
                            key => {
                                // Derive printable ASCII from key + shift.
                                // Apple II convention: letters are always uppercase.
                                any_key = true;
                                if let Some(c) = apple2_ascii_for_key(*key, shift) {
                                    key_queue.push(c);
                                }
                            }
                        }
                    }
                }
            });

            // Any key press exits logo mode and starts the emulator
            if in_logo_mode && any_key {
                self.reset(true);
            }

            // Apply video mode shortcut (Ctrl+1..5)
            if let Some(vt) = video_shortcut {
                self.config.video_type = vt;
                self.renderer.mono_tint = self.config.mono_tint();
                self.config.save();
            }

            // Process clipboard paste text → fill paste_buf.
            // Converts to Apple II ASCII: uppercase letters, CR newlines.
            if let Some(text) = paste_text {
                for ch in text.chars() {
                    let b: u8 = match ch {
                        '\n' | '\r' => 0x0D,
                        c if (' '..='~').contains(&c) => (c as u8).to_ascii_uppercase(),
                        _ => continue,
                    };
                    self.paste_buf.push_back(b);
                }
            }

            if !in_logo_mode {
                for k in key_queue { self.emu.bus.key_press(k); }

                // Drain one character per frame from the paste buffer.
                // Only inject when the keyboard strobe has been cleared (bit 7 == 0),
                // which means the previous key has been read by the Apple II.
                if self.emu.bus.keyboard_data & 0x80 == 0
                    && let Some(k) = self.paste_buf.pop_front()
                {
                    self.emu.bus.key_press(k);
                }

                // ── Joystick / paddle emulation ──────────────────────────────
                // Arrow-key joystick: update paddles each frame from key-hold state.
                use crate::config::JoystickType;
                if self.config.joystick0_type == JoystickType::KeypadArrows {
                    let (lx, rx, uy, dy, b0) = ctx.input(|i| (
                        i.key_down(Key::ArrowLeft),
                        i.key_down(Key::ArrowRight),
                        i.key_down(Key::ArrowUp),
                        i.key_down(Key::ArrowDown),
                        i.modifiers.alt,
                    ));
                    let center = self.config.joystick_self_centering;
                    self.emu.bus.gamepad.paddle0 = if lx { 0 } else if rx { 255 } else if center { 127 } else { self.emu.bus.gamepad.paddle0 };
                    self.emu.bus.gamepad.paddle1 = if uy { 0 } else if dy { 255 } else if center { 127 } else { self.emu.bus.gamepad.paddle1 };
                    let btn = if self.config.joystick_swap_buttons { 0x02u8 } else { 0x01u8 };
                    if b0 { self.emu.bus.gamepad.buttons |=  btn; }
                    else  { self.emu.bus.gamepad.buttons &= !btn; }
                }
            }

            // Screenshot: save framebuffer as BMP (triggered by F12).
            if take_screenshot && !in_logo_mode {
                self.render_apple2(); // ensure latest frame
                save_screenshot(&self.pixel_buf, SCREEN_W, SCREEN_H);
            }

            if do_hard_reset { self.reset(true); }
            if do_reset      { self.reset(false); }
            if do_quit       { frame.close(); return; }

            // ── Render display to GPU texture ─────────────────────────────────
            // When the debugger is active, render the debugger display into the
            // framebuffer instead of the Apple II screen — exactly like the
            // original AppleWin, where the debugger replaces the screen.
            if !in_logo_mode {
                if self.show_debugger && self.debugger.active {
                    self.render_debugger();
                } else {
                    self.render_apple2();
                }
                let tex_opts = TextureOptions {
                    magnification: egui::TextureFilter::Nearest,
                    minification:  egui::TextureFilter::Nearest,
                };
                let image = ColorImage::from_rgba_unmultiplied([SCREEN_W, SCREEN_H], &self.pixel_buf);
                if let Some(t) = &mut self.texture {
                    t.set(image, tex_opts);
                } else {
                    self.texture = Some(ctx.load_texture("apple2", image, tex_opts));
                }
            }
            let tex_id = if in_logo_mode {
                self.logo_texture.as_ref().map(|t| t.id())
            } else {
                self.texture.as_ref().map(|t| t.id())
            };

            // Snapshot emulator state for use in closures (avoids borrow conflicts)
            let pc       = self.emu.cpu.pc;
            let cycles   = self.emu.cpu.cycles;
            let d1_name  = Self::disk_display_name(&self.disk1).to_owned();
            let d2_name  = Self::disk_display_name(&self.disk2).to_owned();
            let d1_loaded = self.disk1.is_some();
            let d2_loaded = self.disk2.is_some();

            // ── Deferred actions (set by panel closures, applied after) ───────
            let mut act_hard_reset    = false;
            let mut act_reset         = false;
            let mut act_quit          = false;
            let mut act_load_disk1    = false;
            let mut act_load_disk2    = false;
            let mut act_eject_disk1   = false;
            let mut act_eject_disk2   = false;
            let mut act_swap          = false;
            let mut act_fullscreen    = false;
            let mut act_about         = false;
            let mut act_show_settings = false;
            let mut act_screenshot    = false;
            let mut act_load_hdd1     = false;
            let mut act_load_hdd2     = false;
            let mut act_eject_hdd1    = false;
            let mut act_eject_hdd2    = false;
            let mut act_recent_disk: Option<String> = None;
            let mut act_recent_hdd: Option<String> = None;

            // ── Menu bar ──────────────────────────────────────────────────────
            egui::TopBottomPanel::top("menubar")
                .frame(
                    egui::Frame::none()
                        .fill(WIN_FACE)
                        .inner_margin(egui::style::Margin::symmetric(4.0, 2.0)),
                )
                .show(ctx, |ui| {
                    egui::menu::bar(ui, |ui| {
                        ui.menu_button("File", |ui| {
                            // ── Floppy disks ─────────────────────────────────
                            if ui.button("Load Disk 1…").clicked() {
                                act_load_disk1 = true; ui.close_menu();
                            }
                            if ui.button("Load Disk 2…").clicked() {
                                act_load_disk2 = true; ui.close_menu();
                            }
                            if ui.add_enabled(d1_loaded, egui::Button::new("Eject Disk 1")).clicked() {
                                act_eject_disk1 = true; ui.close_menu();
                            }
                            if ui.add_enabled(d2_loaded, egui::Button::new("Eject Disk 2")).clicked() {
                                act_eject_disk2 = true; ui.close_menu();
                            }
                            // ── Recent Disks submenu ──────────────────────────
                            ui.menu_button("Recent Disks", |ui| {
                                if self.config.recent_disks.is_empty() {
                                    ui.label("(none)");
                                } else {
                                    for path in self.config.recent_disks.clone() {
                                        let name = std::path::Path::new(&path)
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .into_owned();
                                        if ui.button(name).clicked() {
                                            act_recent_disk = Some(path);
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                            ui.separator();
                            // ── HDD images ───────────────────────────────────
                            {
                                let hdd1_name = self.config.last_hdd1.as_deref()
                                    .and_then(|p| std::path::Path::new(p).file_name())
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "(none)".to_string());
                                let hdd2_name = self.config.last_hdd2.as_deref()
                                    .and_then(|p| std::path::Path::new(p).file_name())
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "(none)".to_string());
                                let hdd1_loaded = self.config.last_hdd1.is_some();
                                let hdd2_loaded = self.config.last_hdd2.is_some();
                                if ui.button(format!("Load HDD 1…  [{}]", hdd1_name)).clicked() {
                                    act_load_hdd1 = true; ui.close_menu();
                                }
                                if ui.add_enabled(hdd1_loaded, egui::Button::new("Eject HDD 1")).clicked() {
                                    act_eject_hdd1 = true; ui.close_menu();
                                }
                                if ui.button(format!("Load HDD 2…  [{}]", hdd2_name)).clicked() {
                                    act_load_hdd2 = true; ui.close_menu();
                                }
                                if ui.add_enabled(hdd2_loaded, egui::Button::new("Eject HDD 2")).clicked() {
                                    act_eject_hdd2 = true; ui.close_menu();
                                }
                            }
                            // ── Recent HDDs submenu ───────────────────────────
                            ui.menu_button("Recent HDDs", |ui| {
                                if self.config.recent_hdds.is_empty() {
                                    ui.label("(none)");
                                } else {
                                    for path in self.config.recent_hdds.clone() {
                                        let name = std::path::Path::new(&path)
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .into_owned();
                                        if ui.button(name).clicked() {
                                            act_recent_hdd = Some(path);
                                            ui.close_menu();
                                        }
                                    }
                                }
                            });
                            ui.separator();
                            if ui.button("Screenshot       F12").clicked() {
                                act_screenshot = true; ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Exit").clicked() {
                                act_quit = true; ui.close_menu();
                            }
                        });
                        ui.menu_button("Machine", |ui| {
                            if ui.button("Reset          Ctrl+F2").clicked() {
                                act_reset = true; ui.close_menu();
                            }
                            if ui.button("Hard Reset          F1").clicked() {
                                act_hard_reset = true; ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Settings…").clicked() {
                                act_show_settings = true; ui.close_menu();
                            }
                        });
                        ui.menu_button("View", |ui| {
                            let label = if self.fullscreen { "Exit Fullscreen  F11" }
                                        else              { "Fullscreen       F11" };
                            if ui.button(label).clicked() {
                                act_fullscreen = true; ui.close_menu();
                            }
                            ui.separator();
                            // ── Video Mode submenu ────────────────────────────
                            ui.menu_button("Video Mode", |ui| {
                                let modes: &[(VideoType, &str)] = &[
                                    (VideoType::ColorTV,          "Color (NTSC TV)      Ctrl+1"),
                                    (VideoType::ColorIdealized,   "Color (Composite)    Ctrl+2"),
                                    (VideoType::ColorRGB,         "RGB                  Ctrl+3"),
                                    (VideoType::MonoWhite,        "Monochrome (white)   Ctrl+4"),
                                    (VideoType::MonoGreen,        "Monochrome (green)   Ctrl+5"),
                                    (VideoType::MonoAmber,        "Monochrome (amber)"),
                                    (VideoType::MonoTV,           "Monochrome TV"),
                                    (VideoType::ColorMonitorNtsc, "Color Monitor NTSC"),
                                    (VideoType::MonoCustom,       "Monochrome (custom)"),
                                ];
                                let current_vt = self.config.video_type;
                                let mut chosen: Option<VideoType> = None;
                                for &(mode, label) in modes {
                                    let text = if current_vt == mode {
                                        format!("✓ {label}")
                                    } else {
                                        format!("  {label}")
                                    };
                                    if ui.button(text).clicked() {
                                        chosen = Some(mode);
                                        ui.close_menu();
                                    }
                                }
                                if let Some(mode) = chosen {
                                    self.config.video_type = mode;
                                    self.renderer.mono_tint = self.config.mono_tint();
                                    self.config.save();
                                }
                            });
                            ui.separator();
                            // ── Display toggles ───────────────────────────────
                            if ui.checkbox(&mut self.config.scanlines, "Scanlines").changed() {
                                self.renderer.scanlines = self.config.scanlines;
                                self.config.save();
                            }
                            if ui.checkbox(&mut self.config.color_vertical_blend, "Colour vertical blend").changed() {
                                self.renderer.color_vertical_blend = self.config.color_vertical_blend;
                                self.config.save();
                            }
                            {
                                let mut is_50hz = self.config.video_refresh_hz == 50;
                                if ui.checkbox(&mut is_50hz, "50 Hz (PAL) mode").changed() {
                                    self.config.video_refresh_hz = if is_50hz { 50 } else { 60 };
                                    self.config.save();
                                }
                            }
                        });
                        ui.menu_button("Help", |ui| {
                            if ui.button("About AppleWin-rs…").clicked() {
                                act_about = true; ui.close_menu();
                            }
                        });
                    });
                });

            // ── Status bar ────────────────────────────────────────────────────
            egui::TopBottomPanel::bottom("statusbar")
                .frame(
                    egui::Frame::none()
                        .fill(WIN_FACE)
                        .stroke(Stroke::new(1.0, WIN_SHADOW))
                        .inner_margin(egui::style::Margin::symmetric(6.0, 3.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        if self.config.show_disk_status {
                            disk_led(ui, "D1", d1_loaded, false);
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(
                                    if d1_loaded { d1_name.as_str() } else { "—" }
                                ).small().monospace(),
                            );
                            ui.add_space(8.0);
                            disk_led(ui, "D2", d2_loaded, false);
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(
                                    if d2_loaded { d2_name.as_str() } else { "—" }
                                ).small().monospace(),
                            );
                        }
                        ui.separator();
                        if in_logo_mode {
                            ui.label(RichText::new("AppleWin-rs — Press any key to start").small());
                        } else {
                            ui.label(RichText::new(format!("PC:${pc:04X}")).small().monospace());
                            ui.add_space(6.0);
                            ui.label(RichText::new(format!("Cyc:{cycles}")).small().monospace());
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new("F1:Reset  Ctrl+Esc:Quit").small());
                        });
                    });
                });

            // ── Right button strip ────────────────────────────────────────────
            let icons = self.icons.as_ref();
            egui::SidePanel::right("buttons")
                .exact_width(BTN_PANEL_W)
                .resizable(false)
                .frame(
                    egui::Frame::none()
                        .fill(WIN_FACE)
                        .stroke(Stroke::new(1.0, WIN_SHADOW))
                        .inner_margin(egui::style::Margin::same(5.0)),
                )
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        let sz = Vec2::new(43.0, 41.0);
                        let isz = Vec2::new(41.0, 41.0);
                        ui.add_space(2.0);
                        if icon_btn(ui, icons.and_then(|ic| ic.help.as_ref()),  "?",  "Help / About",       sz, isz).clicked() { act_about = true; }
                        ui.add_space(2.0);
                        // Matches AppleWin BTN_RUN logic:
                        //   Ctrl+click → CtrlReset (soft reset, warm CPU)
                        //   click      → ResetMachineState (power cycle)
                        if icon_btn(ui, icons.and_then(|ic| ic.run.as_ref()), "↺",
                                    "Reset  (Ctrl+click = soft reset, click = power cycle)",
                                    sz, isz).clicked() {
                            let ctrl = ui.ctx().input(|i| i.modifiers.ctrl || i.modifiers.command);
                            if ctrl { act_reset = true; } else { act_hard_reset = true; }
                        }
                        ui.add_space(2.0);
                        if icon_btn(ui, icons.and_then(|ic| ic.d1.as_ref()),    "①", "Load Disk 1",         sz, isz).clicked() { act_load_disk1 = true; }
                        ui.add_space(2.0);
                        if icon_btn(ui, icons.and_then(|ic| ic.d2.as_ref()),    "②", "Load Disk 2",         sz, isz).clicked() { act_load_disk2 = true; }
                        ui.add_space(2.0);
                        if icon_btn(ui, icons.and_then(|ic| ic.swap.as_ref()),  "⇄",  "Swap Drives",         sz, isz).clicked() { act_swap = true; }
                        ui.add_space(2.0);
                        if icon_btn(ui, icons.and_then(|ic| ic.full.as_ref()),  "⛶",  "Fullscreen (F11)",    sz, isz).clicked() { act_fullscreen = true; }
                        ui.add_space(2.0);
                        if icon_btn(ui, icons.and_then(|ic| ic.debug.as_ref()), "⚙", "Debugger", sz, isz).clicked() {
                            self.show_debugger = !self.show_debugger;
                        }
                        ui.add_space(2.0);
                        if icon_btn(ui, icons.and_then(|ic| ic.setup.as_ref()), "⚙",  "Settings",            sz, isz).clicked() { act_show_settings = true; }
                    });
                });

            // ── Confirm reboot dialog ─────────────────────────────────────────
            if let Some(power_cycle) = self.pending_reset {
                let mut do_reset  = false;
                let mut do_cancel = false;
                let label = if power_cycle { "Hard Reset (power cycle)" } else { "Reset" };
                egui::Window::new("Confirm Reset")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.add_space(4.0);
                        ui.label(format!("Are you sure you want to {label}?"));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("  OK  ").clicked()     { do_reset  = true; }
                            if ui.button("Cancel").clicked() { do_cancel = true; }
                        });
                    });
                if do_reset  { self.reset(power_cycle); self.pending_reset = None; }
                if do_cancel { self.pending_reset = None; }
            }

            // ── Debugger ──────────────────────────────────────────────────────
            // Renders directly into the 560×384 framebuffer (replacing the
            // Apple II screen), matching the original AppleWin debugger.
            // Keyboard input is captured via egui events; a small command-
            // input overlay appears at the bottom of the main screen area.
            if self.show_debugger && self.debugger.active {
                use apple2_debugger::disasm::{disassemble_one, format_instruction};
                use apple2_debugger::commands::{self, CmdResult, CpuRegs};

                // The debugger display is rendered into the framebuffer by
                // render_debugger() above. Here we only handle keyboard
                // input and process commands.
                let paused = self.debugger.active;

                let mut do_step      = false;
                let mut do_step_over = false;
                let mut do_step_out  = false;
                let mut do_resume    = false;
                let mut do_exec_cmd  = false;

                // Keyboard shortcuts (when no text field has focus)
                ctx.input(|i| {
                    for event in &i.events {
                        if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event {
                            match key {
                                Key::F5  => do_resume = true,     // Go
                                Key::F10 => do_step_over = true,  // Step Over
                                Key::F11 => do_step = true,       // Step Into
                                Key::F7 if modifiers.shift => do_step_out = true,
                                _ => {}
                            }
                        }
                    }
                });

                // Small command input bar overlaid at bottom of screen area
                egui::TopBottomPanel::bottom("dbg_cmd")
                    .frame(
                        egui::Frame::none()
                            .fill(Color32::from_rgb(16, 16, 32))
                            .inner_margin(egui::style::Margin::symmetric(6.0, 2.0)),
                    )
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            if paused {
                                if ui.button("G").clicked() { do_resume = true; }
                                if ui.button("S").clicked() { do_step = true; }
                                if ui.button("SO").clicked() { do_step_over = true; }
                                if ui.button("OUT").clicked() { do_step_out = true; }
                            }
                            ui.label(RichText::new(">").monospace().color(Color32::from_rgb(255, 128, 0)));
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.debugger_cmd_input)
                                    .desired_width(ui.available_width() - 8.0)
                                    .font(FontId::monospace(12.0))
                                    .text_color(Color32::WHITE)
                            );
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                do_exec_cmd = true;
                                resp.request_focus();
                            }
                        });
                    });

                // ── Apply deferred actions ─────────────────────────────────
                if do_resume {
                    self.debugger.deactivate();
                }
                if do_step && paused {
                    if self.debugger.trace.enabled {
                        use apple2_debugger::trace::TraceEntry;
                        let instr = disassemble_one(self.emu.cpu.pc, |a| self.emu.bus.read_raw(a));
                        self.debugger.trace.push(TraceEntry {
                            pc: self.emu.cpu.pc,
                            opcode: instr.opcode,
                            a: self.emu.cpu.a,
                            x: self.emu.cpu.x,
                            y: self.emu.cpu.y,
                            sp: self.emu.cpu.sp,
                            flags: self.emu.cpu.flags.bits(),
                            cycles: self.emu.cpu.cycles,
                            text: format_instruction(&instr),
                        });
                    }
                    self.emu.step();
                    self.debugger.stop_reason = apple2_debugger::state::StopReason::Step;
                }
                if do_step_over && paused {
                    let opcode = self.emu.bus.read_raw(self.emu.cpu.pc);
                    if opcode == 0x20 {
                        self.debugger.step_over_target = Some(self.emu.cpu.pc.wrapping_add(3));
                        self.debugger.deactivate();
                    } else {
                        self.emu.step();
                        self.debugger.stop_reason = apple2_debugger::state::StopReason::StepOver;
                    }
                }
                if do_step_out && paused {
                    self.debugger.step_out_sp = Some(self.emu.cpu.sp);
                    self.debugger.deactivate();
                }

                // Execute console command
                if do_exec_cmd {
                    let cmd_text = self.debugger_cmd_input.clone();
                    self.debugger_cmd_input.clear();
                    self.debugger.print(format!("> {cmd_text}"));

                    let regs = CpuRegs {
                        a: self.emu.cpu.a,
                        x: self.emu.cpu.x,
                        y: self.emu.cpu.y,
                        sp: self.emu.cpu.sp,
                        pc: self.emu.cpu.pc,
                        flags: self.emu.cpu.flags.bits(),
                        cycles: self.emu.cpu.cycles,
                    };
                    let result = commands::execute_command(
                        &mut self.debugger,
                        &cmd_text,
                        self.emu.cpu.pc,
                        regs,
                        |a| self.emu.bus.read_raw(a),
                    );
                    match result {
                        CmdResult::Output(lines) => {
                            self.debugger.print_lines(&lines);
                        }
                        CmdResult::Go => {
                            self.debugger.deactivate();
                        }
                        CmdResult::Step => {
                            self.emu.step();
                            self.debugger.stop_reason = apple2_debugger::state::StopReason::Step;
                        }
                        CmdResult::StepOver => {
                            let opcode = self.emu.bus.read_raw(self.emu.cpu.pc);
                            if opcode == 0x20 {
                                self.debugger.step_over_target = Some(self.emu.cpu.pc.wrapping_add(3));
                                self.debugger.deactivate();
                            } else {
                                self.emu.step();
                            }
                        }
                        CmdResult::StepOut => {
                            self.debugger.step_out_sp = Some(self.emu.cpu.sp);
                            self.debugger.deactivate();
                        }
                        CmdResult::Trace(n) => {
                            self.debugger.trace.enabled = true;
                            self.debugger.trace_remaining = n;
                        }
                        CmdResult::SetPC(addr) => {
                            self.emu.cpu.pc = addr;
                            self.debugger.deactivate();
                        }
                        CmdResult::MemWrite(addr, val) => {
                            self.emu.bus.write_raw(addr, val);
                            self.debugger.print(format!("  ${addr:04X} = {val:02X}"));
                        }
                        CmdResult::SetReg(reg, val) => {
                            match reg {
                                'A' => self.emu.cpu.a = val as u8,
                                'X' => self.emu.cpu.x = val as u8,
                                'Y' => self.emu.cpu.y = val as u8,
                                'S' | 'P' if val > 0xFF => self.emu.cpu.pc = val,
                                'S' => self.emu.cpu.sp = val as u8,
                                'P' => self.emu.cpu.pc = val,
                                _ => {}
                            }
                        }
                        CmdResult::Nop => {}
                        CmdResult::Error(msg) => {
                            self.debugger.print(format!("Error: {msg}"));
                        }
                        CmdResult::ToggleBreak => {
                            if self.debugger.active {
                                self.debugger.deactivate();
                            } else {
                                self.debugger.activate(apple2_debugger::state::StopReason::UserBreak);
                            }
                        }
                    }
                }
            }

            // ── Settings dialog ───────────────────────────────────────────────
            if self.show_settings {
                let mut apply_settings  = false;
                let mut cancel_settings = false;
                egui::Window::new("Settings")
                    .collapsible(false)
                    .resizable(false)
                    .min_width(340.0)
                    .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        // ── Tab bar ───────────────────────────────────────────
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.settings_tab, 0, "Machine");
                            ui.selectable_value(&mut self.settings_tab, 1, "Video");
                            ui.selectable_value(&mut self.settings_tab, 2, "Audio");
                            ui.selectable_value(&mut self.settings_tab, 3, "Speed");
                            ui.selectable_value(&mut self.settings_tab, 4, "Input");
                            ui.selectable_value(&mut self.settings_tab, 5, "Slots");
                            ui.selectable_value(&mut self.settings_tab, 6, "Advanced");
                        });
                        ui.separator();

                        // ── Tab contents ──────────────────────────────────────
                        match self.settings_tab {
                            0 => {
                                // Machine tab
                                egui::Grid::new("machine_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label("Computer type:");
                                        egui::ComboBox::from_id_source("machine_type")
                                            .selected_text(model_name(self.pending_config.machine_type))
                                            .show_ui(ui, |ui| {
                                                for m in [
                                                    Apple2Model::AppleII,
                                                    Apple2Model::AppleIIPlus,
                                                    Apple2Model::AppleIIe,
                                                    Apple2Model::AppleIIeEnh,
                                                    Apple2Model::AppleIIc,
                                                ] {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.machine_type,
                                                        m,
                                                        model_name(m),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                        ui.label("CPU type:");
                                        egui::ComboBox::from_id_source("cpu_type")
                                            .selected_text(cpu_name(self.pending_config.cpu_type))
                                            .show_ui(ui, |ui| {
                                                for c in [CpuType::Cpu6502, CpuType::Cpu65C02, CpuType::CpuZ80] {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.cpu_type,
                                                        c,
                                                        cpu_name(c),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                    });
                                if self.pending_config.machine_type != self.config.machine_type
                                    || self.pending_config.cpu_type != self.config.cpu_type
                                {
                                    ui.add_space(4.0);
                                    ui.colored_label(
                                        Color32::from_rgb(180, 100, 0),
                                        "⚠ Machine change requires a hard reset.",
                                    );
                                }
                            }
                            1 => {
                                // Video tab
                                egui::Grid::new("video_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label("Video type:");
                                        egui::ComboBox::from_id_source("video_type")
                                            .selected_text(video_type_name(self.pending_config.video_type))
                                            .show_ui(ui, |ui| {
                                                for &vt in ALL_VIDEO_TYPES {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.video_type,
                                                        vt,
                                                        video_type_name(vt),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                        // Monochrome custom colour picker
                                        if self.pending_config.video_type == VideoType::MonoCustom {
                                            ui.label("Mono colour:");
                                            let c = self.pending_config.monochrome_color;
                                            let mut rgb = [
                                                ((c >> 16) & 0xFF) as u8,
                                                ((c >>  8) & 0xFF) as u8,
                                                ( c        & 0xFF) as u8,
                                            ];
                                            if ui.color_edit_button_srgb(&mut rgb).changed() {
                                                self.pending_config.monochrome_color =
                                                    ((rgb[0] as u32) << 16) |
                                                    ((rgb[1] as u32) <<  8) |
                                                     (rgb[2] as u32);
                                            }
                                            ui.end_row();
                                        }
                                        ui.label("Refresh rate:");
                                        ui.horizontal(|ui| {
                                            ui.radio_value(&mut self.pending_config.video_refresh_hz, 60, "60 Hz (NTSC)");
                                            ui.radio_value(&mut self.pending_config.video_refresh_hz, 50, "50 Hz (PAL)");
                                        });
                                        ui.end_row();
                                    });
                                ui.add_space(4.0);
                                ui.checkbox(&mut self.pending_config.scanlines, "CRT scanlines (half-scanline darkening)");
                                ui.checkbox(&mut self.pending_config.color_vertical_blend, "Colour vertical blend");
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(format!(
                                        "Cycles per frame: {}",
                                        self.pending_config.cycles_per_frame()
                                    ))
                                    .small(),
                                );
                            }
                            2 => {
                                // Audio tab
                                ui.add_space(4.0);
                                ui.add(
                                    egui::Slider::new(&mut self.pending_config.master_volume, 0..=100)
                                        .text("Master volume")
                                        .suffix("%"),
                                );
                            }
                            3 => {
                                // Speed tab
                                const SPEED_NORMAL: u32 = 10; // 1.023 MHz
                                ui.add_space(4.0);
                                let mhz = self.pending_config.emulation_speed as f64 * 0.1023;
                                let speed_label = if self.pending_config.emulation_speed == SPEED_NORMAL {
                                    "CPU speed  (Authentic / Normal)".to_string()
                                } else {
                                    format!("CPU speed  ({:.2} MHz)", mhz)
                                };
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::Slider::new(&mut self.pending_config.emulation_speed, 1..=40)
                                            .text(speed_label),
                                    );
                                    if self.pending_config.emulation_speed != SPEED_NORMAL
                                        && ui.small_button("Reset to normal").clicked() {
                                        self.pending_config.emulation_speed = SPEED_NORMAL;
                                    }
                                });
                                ui.add_space(4.0);
                                ui.checkbox(
                                    &mut self.pending_config.enhanced_disk_speed,
                                    "Enhanced disk speed  (16× while motor is spinning)",
                                );
                                ui.add_space(4.0);
                                ui.label(RichText::new(
                                    "Speeds up disk-based game boot times significantly."
                                ).small());
                            }
                            4 => {
                                // Input tab
                                egui::Grid::new("input_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label("Joystick 1:");
                                        egui::ComboBox::from_id_source("joy0_type")
                                            .selected_text(joystick_type_name(self.pending_config.joystick0_type))
                                            .show_ui(ui, |ui| {
                                                for &jt in ALL_JOYSTICK_TYPES {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.joystick0_type,
                                                        jt,
                                                        joystick_type_name(jt),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                        ui.label("Joystick 2:");
                                        egui::ComboBox::from_id_source("joy1_type")
                                            .selected_text(joystick_type_name(self.pending_config.joystick1_type))
                                            .show_ui(ui, |ui| {
                                                for &jt in ALL_JOYSTICK_TYPES {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.joystick1_type,
                                                        jt,
                                                        joystick_type_name(jt),
                                                    );
                                                }
                                            });
                                        ui.end_row();
                                    });
                                ui.add_space(4.0);
                                ui.checkbox(&mut self.pending_config.joystick_swap_buttons,    "Swap joystick buttons");
                                ui.checkbox(&mut self.pending_config.joystick_autofire,         "Auto-fire button 0");
                                ui.checkbox(&mut self.pending_config.joystick_self_centering,   "Self-centring joystick");
                                ui.checkbox(&mut self.pending_config.joystick_cursor_control,   "Cursor keys control joystick");
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(4.0);
                                ui.checkbox(&mut self.pending_config.mouse_crosshair,           "Show crosshair mouse cursor");
                                ui.checkbox(&mut self.pending_config.mouse_restrict_to_window,  "Restrict mouse to window");
                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(4.0);
                                egui::Grid::new("paddle_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Paddle X trim:");
                                        let mut px = self.pending_config.paddle_x_trim as i32;
                                        if ui.add(egui::Slider::new(&mut px, -128..=127)).changed() {
                                            self.pending_config.paddle_x_trim = px as i8;
                                        }
                                        ui.end_row();
                                        ui.label("Paddle Y trim:");
                                        let mut py = self.pending_config.paddle_y_trim as i32;
                                        if ui.add(egui::Slider::new(&mut py, -128..=127)).changed() {
                                            self.pending_config.paddle_y_trim = py as i8;
                                        }
                                        ui.end_row();
                                    });
                                ui.add_space(4.0);
                                ui.label(RichText::new(
                                    "Joystick emulation is not yet implemented.\n\
                                     Settings are saved for future use."
                                ).small());
                            }
                            5 => {
                                // Slots tab
                                ui.add_space(4.0);
                                egui::Grid::new("slots_grid")
                                    .num_columns(3)
                                    .spacing([12.0, 4.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        for slot in 0..8usize {
                                            ui.label(format!("Slot {slot}:"));
                                            let current = self.pending_config.slot_cards[slot];
                                            egui::ComboBox::from_id_source(format!("slot_{slot}"))
                                                .selected_text(card_name(current))
                                                .show_ui(ui, |ui| {
                                                    for &card in IMPLEMENTED_CARDS {
                                                        ui.selectable_value(
                                                            &mut self.pending_config.slot_cards[slot],
                                                            card,
                                                            card_name(card),
                                                        );
                                                    }
                                                });
                                            // Options button — only for cards with configurable options
                                            let has_options = matches!(
                                                current,
                                                CardType::Mockingboard
                                                | CardType::Phasor
                                                | CardType::Saturn128K
                                                | CardType::RamWorksIII
                                            );
                                            if has_options {
                                                if ui.small_button("Options…").clicked() {
                                                    self.slot_options_open[slot] = true;
                                                }
                                            } else {
                                                ui.label(""); // spacer
                                            }
                                            ui.end_row();
                                        }
                                        // Aux slot row
                                        ui.label("Aux slot:");
                                        let aux_current = self.pending_config.aux_slot_card;
                                        egui::ComboBox::from_id_source("slot_aux")
                                            .selected_text(card_name(aux_current))
                                            .show_ui(ui, |ui| {
                                                for &card in &[
                                                    CardType::Empty,
                                                    CardType::Extended80Col,
                                                    CardType::Col80,
                                                    CardType::RamWorksIII,
                                                ] {
                                                    ui.selectable_value(
                                                        &mut self.pending_config.aux_slot_card,
                                                        card,
                                                        card_name(card),
                                                    );
                                                }
                                            });
                                        ui.label(""); // spacer (no options button for aux)
                                        ui.end_row();
                                    });
                                ui.add_space(4.0);
                                ui.label(RichText::new(
                                    "Slot changes take effect after OK (requires reset)."
                                ).small());
                            }
                            6 => {
                                // Advanced tab
                                ui.add_space(4.0);
                                ui.label("Display:");
                                egui::Grid::new("adv_display_grid")
                                    .num_columns(2)
                                    .spacing([12.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("Window scale:");
                                        ui.add(
                                            egui::Slider::new(
                                                &mut self.pending_config.window_scale,
                                                1..=4,
                                            )
                                            .suffix("×"),
                                        );
                                        ui.end_row();
                                    });
                                ui.checkbox(
                                    &mut self.pending_config.show_disk_status,
                                    "Show Disk II activity LEDs",
                                );
                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(4.0);
                                ui.label("Behaviour:");
                                ui.checkbox(
                                    &mut self.pending_config.confirm_reboot,
                                    "Confirm before reset",
                                );
                                ui.checkbox(
                                    &mut self.pending_config.scrolllock_toggle,
                                    "F10 key toggles pause (ScrollLock equivalent)",
                                );
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(4.0);
                                ui.label("Save state:");
                                ui.checkbox(
                                    &mut self.pending_config.save_state_on_exit,
                                    "Save state on exit / restore on launch",
                                );
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label("File:");
                                    let fname = self.pending_config.save_state_filename
                                        .as_deref()
                                        .unwrap_or("(auto)");
                                    ui.label(RichText::new(fname).monospace().small());
                                    if ui.small_button("Browse…").clicked()
                                        && let Some(path) = rfd::FileDialog::new()
                                            .set_title("Save State File")
                                            .add_filter("AWS YAML", &["yaml", "aws.yaml"])
                                            .save_file()
                                    {
                                        self.pending_config.save_state_filename =
                                            Some(path.to_string_lossy().into_owned());
                                    }
                                    if ui.small_button("Clear").clicked() {
                                        self.pending_config.save_state_filename = None;
                                    }
                                });
                            }
                            _ => {}
                        }

                        // ── Buttons ───────────────────────────────────────────
                        ui.add_space(8.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("  OK  ").clicked()     { apply_settings = true; }
                            if ui.button("Cancel").clicked() { cancel_settings = true; }
                        });
                    });

                if apply_settings {
                    let machine_changed =
                        self.pending_config.machine_type != self.config.machine_type
                        || self.pending_config.cpu_type  != self.config.cpu_type;
                    let slots_changed    = self.pending_config.slot_cards    != self.config.slot_cards;
                    let scale_changed    = self.pending_config.window_scale  != self.config.window_scale;
                    self.config = self.pending_config.clone();
                    // Apply video settings immediately
                    self.renderer.scanlines            = self.config.scanlines;
                    self.renderer.mono_tint            = self.config.mono_tint();
                    self.renderer.color_vertical_blend = self.config.color_vertical_blend;
                    // Resize window if scale changed, but only when not maximized.
                    // In maximized state the OS controls the window size; we leave
                    // it alone so the user can un-maximize and get the right size.
                    if scale_changed && !frame.info().window_info.maximized {
                        let s = self.config.window_scale.max(1) as f32;
                        frame.set_window_size(egui::vec2(
                            SCREEN_W as f32 * s + BEVEL * 2.0 + BTN_PANEL_W + 24.0,
                            SCREEN_H as f32 * s + BEVEL * 2.0 + 80.0,
                        ));
                    }
                    // Rebuild emulator if machine/CPU/slots changed
                    if machine_changed || slots_changed {
                        let disk1 = self.disk1.clone();
                        let disk2 = self.disk2.clone();
                        self.emu = super::make_emulator(
                            self.config.machine_type,
                            self.config.cpu_type,
                        );
                        apply_slot_cards(&mut self.emu, &self.config);
                        self.disk_slot = self.config.disk2_slot();
                        self.emu.mode = apple2_core::emulator::AppMode::Running;
                        Self::reload_disk(&mut self.emu, self.disk_slot, 0, &disk1);
                        Self::reload_disk(&mut self.emu, self.disk_slot, 1, &disk2);
                        self.speaker_state    = false;
                        self.dc_filter_ctr    = 0;
                        self.spkr_cycle_rem   = 0.0;
                        self.last_audio_cycle = self.emu.cpu.cycles;
                    }
                    self.config.save();
                    self.show_settings = false;
                }
                if cancel_settings {
                    self.pending_config = self.config.clone();
                    self.show_settings  = false;
                }
            }

            // ── Per-card options popups ───────────────────────────────────────
            for slot in 0..8usize {
                if !self.slot_options_open[slot] { continue; }
                let card_type = self.pending_config.slot_cards[slot];
                let title = format!("Slot {} — {} Options", slot, card_name(card_type));
                let mut still_open = self.slot_options_open[slot];
                egui::Window::new(title)
                    .collapsible(false)
                    .resizable(false)
                    .open(&mut still_open)
                    .show(ctx, |ui| {
                        match card_type {
                            CardType::Mockingboard => {
                                ui.checkbox(
                                    &mut self.pending_config.mockingboard_has_speech,
                                    "Enable SSI263 speech chips",
                                );
                            }
                            CardType::Phasor => {
                                ui.label("Phasor mode:");
                                ui.radio_value(
                                    &mut self.pending_config.phasor_native_mode,
                                    true,
                                    "Phasor native mode",
                                );
                                ui.radio_value(
                                    &mut self.pending_config.phasor_native_mode,
                                    false,
                                    "Mockingboard compatible mode",
                                );
                            }
                            CardType::Saturn128K => {
                                ui.label("RAM size:");
                                for &kb in &[16u32, 32, 64, 128] {
                                    ui.radio_value(
                                        &mut self.pending_config.saturn_ram_kb,
                                        kb,
                                        format!("{kb}K"),
                                    );
                                }
                            }
                            CardType::RamWorksIII => {
                                ui.label("RAM size:");
                                for &kb in &[64u32, 128, 256, 512, 1024, 2048, 4096, 8192] {
                                    let label = if kb >= 1024 {
                                        format!("{}MB", kb / 1024)
                                    } else {
                                        format!("{kb}K")
                                    };
                                    ui.radio_value(
                                        &mut self.pending_config.ramworks_ram_kb,
                                        kb,
                                        label,
                                    );
                                }
                            }
                            _ => {
                                ui.label("No options available.");
                            }
                        }
                    });
                self.slot_options_open[slot] = still_open;
            }

            // ── About dialog ──────────────────────────────────────────────────
            if self.show_about {
                egui::Window::new("About AppleWin-rs")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.heading("AppleWin-rs");
                            ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                            ui.add_space(4.0);
                            ui.label("Cross-platform Apple II emulator");
                            ui.label("Rust rewrite of AppleWin");
                            ui.add_space(8.0);
                            if ui.button("  OK  ").clicked() {
                                self.show_about = false;
                            }
                        });
                    });
            }

            // ── Central panel — Apple II screen framed with 3D bevel ──────────
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(WIN_FACE)
                        .inner_margin(egui::style::Margin::same(8.0)),
                )
                .show(ctx, |ui| {
                    let avail = ui.available_rect_before_wrap();

                    let sw = SCREEN_W as f32;
                    let sh = SCREEN_H as f32;
                    // Use integer scaling in PHYSICAL pixels, not egui points.
                    // At non-100% DPI (e.g. 125%), an egui-point integer scale maps
                    // to a fractional physical pixel count, causing the GPU to blend
                    // adjacent framebuffer rows even with Nearest filtering.
                    let ppp = ctx.pixels_per_point();
                    let avail_pw = (avail.width()  - BEVEL * 2.0) * ppp;
                    let avail_ph = (avail.height() - BEVEL * 2.0) * ppp;
                    let phys_scale_w = (avail_pw / sw).floor().max(1.0);
                    let phys_scale_h = (avail_ph / sh).floor().max(1.0);
                    let phys_scale   = phys_scale_w.min(phys_scale_h);
                    // Convert back to egui points for layout
                    let scale = phys_scale / ppp;
                    let outer_w = sw * scale + BEVEL * 2.0;
                    let outer_h = sh * scale + BEVEL * 2.0;

                    let ox = avail.left() + ((avail.width()  - outer_w) / 2.0).max(0.0);
                    let oy = avail.top()  + ((avail.height() - outer_h) / 2.0).max(0.0);

                    let outer  = Rect::from_min_size(Pos2::new(ox, oy), Vec2::new(outer_w, outer_h));
                    let screen = outer.shrink(BEVEL);

                    let painter = ui.painter();
                    draw_sunken_bevel(painter, outer);

                    if let Some(tid) = tex_id {
                        painter.image(
                            tid,
                            screen,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }

                    // ── Logo mode overlay — version + prompt ──────────────────
                    if in_logo_mode {
                        let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
                        // Version string position mirrors the C++ DRAWVERSION macro:
                        // scale*540 x scale*358 relative to the 560×384 logo area.
                        let vx = screen.left() + screen.width()  * (540.0 / 560.0);
                        let vy = screen.top()  + screen.height() * (358.0 / 384.0);
                        let vfont = FontId::proportional(13.0);
                        // 3-layer rendering matching C++ hi-colour path:
                        // (+1,+1) dark shadow, (-1,-1) light highlight, (0,0) main purple
                        painter.text(
                            Pos2::new(vx + 1.0, vy + 1.0),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont.clone(),
                            Color32::from_rgb(0x30, 0x30, 0x70),
                        );
                        painter.text(
                            Pos2::new(vx - 1.0, vy - 1.0),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont.clone(),
                            Color32::from_rgb(0xC0, 0x70, 0xE0),
                        );
                        painter.text(
                            Pos2::new(vx, vy),
                            egui::Align2::RIGHT_BOTTOM,
                            &version,
                            vfont,
                            Color32::from_rgb(0x70, 0x30, 0xE0),
                        );
                        // "Press any key" prompt at bottom-centre
                        let px = screen.center().x;
                        let py = screen.bottom() - 10.0;
                        let pfont = FontId::proportional(13.0);
                        painter.text(
                            Pos2::new(px + 1.0, py + 1.0),
                            egui::Align2::CENTER_BOTTOM,
                            "Press any key to start",
                            pfont.clone(),
                            Color32::BLACK,
                        );
                        painter.text(
                            Pos2::new(px, py),
                            egui::Align2::CENTER_BOTTOM,
                            "Press any key to start",
                            pfont,
                            Color32::WHITE,
                        );
                    }

                    ui.allocate_rect(outer, Sense::hover());
                });

            // ── Apply deferred actions ────────────────────────────────────────
            if act_hard_reset {
                if self.config.confirm_reboot {
                    self.pending_reset = Some(true);
                } else {
                    self.reset(true);
                }
            }
            if act_reset {
                if self.config.confirm_reboot {
                    self.pending_reset = Some(false);
                } else {
                    self.reset(false);
                }
            }
            if act_quit        { self.config.save(); frame.close(); }
            if act_about       { self.show_about = true; }
            if act_show_settings {
                self.pending_config = self.config.clone();
                self.show_settings  = true;
            }
            if act_fullscreen  {
                self.fullscreen = !self.fullscreen;
                frame.set_fullscreen(self.fullscreen);
            }
            if act_swap {
                // Swap path names
                std::mem::swap(&mut self.disk1, &mut self.disk2);
                std::mem::swap(&mut self.config.last_disk1, &mut self.config.last_disk2);
                // Re-load both drives so the card reflects the swap
                let d1 = self.disk1.clone();
                let d2 = self.disk2.clone();
                Self::reload_disk(&mut self.emu, self.disk_slot, 0, &d1);
                Self::reload_disk(&mut self.emu, self.disk_slot, 1, &d2);
            }
            if act_eject_disk1 {
                self.emu.bus.eject_disk(self.disk_slot, 0);
                self.disk1 = None;
                self.config.last_disk1 = None;
                self.config.save();
            }
            if act_eject_disk2 {
                self.emu.bus.eject_disk(self.disk_slot, 1);
                self.disk2 = None;
                self.config.last_disk2 = None;
                self.config.save();
            }
            if act_load_disk1 {
                let start_dir = self.config.last_disk_dir.as_deref();
                if let Some(path) = open_disk_dialog("Load Disk 1", start_dir)
                    && let Ok(data) = std::fs::read(&path)
                {
                    let ext = path.extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    self.emu.bus.load_disk(self.disk_slot, 0, &data, &ext);
                    let path_str = path.to_string_lossy().into_owned();
                    self.config.add_recent_disk(&path_str);
                    self.config.last_disk1 = Some(path_str);
                    self.config.last_disk_dir = path.parent()
                        .map(|p| p.to_string_lossy().into_owned());
                    self.disk1 = Some(path);
                    self.config.save();
                }
            }
            if act_load_disk2 {
                let start_dir = self.config.last_disk_dir.as_deref();
                if let Some(path) = open_disk_dialog("Load Disk 2", start_dir)
                    && let Ok(data) = std::fs::read(&path)
                {
                    let ext = path.extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    self.emu.bus.load_disk(self.disk_slot, 1, &data, &ext);
                    let path_str = path.to_string_lossy().into_owned();
                    self.config.add_recent_disk(&path_str);
                    self.config.last_disk2 = Some(path_str);
                    self.config.last_disk_dir = path.parent()
                        .map(|p| p.to_string_lossy().into_owned());
                    self.disk2 = Some(path);
                    self.config.save();
                }
            }
            // Load disk from recent list into drive 1
            if let Some(path_str) = act_recent_disk {
                let path = PathBuf::from(&path_str);
                if let Ok(data) = std::fs::read(&path) {
                    let ext = path.extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    self.emu.bus.load_disk(self.disk_slot, 0, &data, &ext);
                    self.emu.bus.set_disk_path(self.disk_slot, 0, path.clone());
                    self.config.add_recent_disk(&path_str);
                    self.config.last_disk1 = Some(path_str);
                    self.config.last_disk_dir = path.parent()
                        .map(|p| p.to_string_lossy().into_owned());
                    self.disk1 = Some(path);
                    self.config.save();
                }
            }
            // HDD: load / eject
            if act_load_hdd1 {
                let start_dir = self.config.last_hdd_dir.as_deref();
                if let Some(path) = open_hdd_dialog("Load HDD 1", start_dir)
                    && let Ok(data) = std::fs::read(&path)
                {
                    let path_str = path.to_string_lossy().into_owned();
                    self.config.add_recent_hdd(&path_str);
                    self.config.last_hdd1 = Some(path_str);
                    self.config.last_hdd_dir = path.parent()
                        .map(|p| p.to_string_lossy().into_owned());
                    // Apply to any installed HD card
                    for slot in 0..apple2_core::card::NUM_SLOTS {
                        if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                            && card.card_type() == apple2_core::card::CardType::GenericHdd
                        {
                            if let Some(hd) = card.as_any_mut()
                                .downcast_mut::<apple2_core::cards::hd::HdCard>()
                            {
                                hd.load_image(0, data);
                            }
                            break;
                        }
                    }
                    self.config.save();
                }
            }
            if act_load_hdd2 {
                let start_dir = self.config.last_hdd_dir.as_deref();
                if let Some(path) = open_hdd_dialog("Load HDD 2", start_dir)
                    && let Ok(data) = std::fs::read(&path)
                {
                    let path_str = path.to_string_lossy().into_owned();
                    self.config.add_recent_hdd(&path_str);
                    self.config.last_hdd2 = Some(path_str);
                    self.config.last_hdd_dir = path.parent()
                        .map(|p| p.to_string_lossy().into_owned());
                    for slot in 0..apple2_core::card::NUM_SLOTS {
                        if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                            && card.card_type() == apple2_core::card::CardType::GenericHdd
                        {
                            if let Some(hd) = card.as_any_mut()
                                .downcast_mut::<apple2_core::cards::hd::HdCard>()
                            {
                                hd.load_image(1, data);
                            }
                            break;
                        }
                    }
                    self.config.save();
                }
            }
            if act_eject_hdd1 {
                self.config.last_hdd1 = None;
                self.config.save();
            }
            if act_eject_hdd2 {
                self.config.last_hdd2 = None;
                self.config.save();
            }
            if let Some(path_str) = act_recent_hdd {
                let path = PathBuf::from(&path_str);
                if let Ok(data) = std::fs::read(&path) {
                    self.config.add_recent_hdd(&path_str);
                    self.config.last_hdd1 = Some(path_str);
                    self.config.last_hdd_dir = path.parent()
                        .map(|p| p.to_string_lossy().into_owned());
                    for slot in 0..apple2_core::card::NUM_SLOTS {
                        if let Some(card) = self.emu.bus.cards.slot_mut(slot)
                            && card.card_type() == apple2_core::card::CardType::GenericHdd
                        {
                            if let Some(hd) = card.as_any_mut()
                                .downcast_mut::<apple2_core::cards::hd::HdCard>()
                            {
                                hd.load_image(0, data);
                            }
                            break;
                        }
                    }
                    self.config.save();
                }
            }

            // Screenshot from menu or F12 key
            if act_screenshot && !in_logo_mode {
                self.render_apple2();
                save_screenshot(&self.pixel_buf, SCREEN_W, SCREEN_H);
            }

            // F11 fullscreen shortcut (supplement to the action already handled above)
            let f11 = ctx.input(|i| i.key_pressed(Key::F11));
            if f11 {
                self.fullscreen = !self.fullscreen;
                frame.set_fullscreen(self.fullscreen);
            }

            // Track window state for persistence (restored on next launch).
            // window_scale is NOT tracked here — it only changes via the Settings
            // dialog, so the saved value always reflects a deliberate user choice
            // rather than an accidental resize.
            let maximized = frame.info().window_info.maximized;
            self.config.window_maximized = maximized;
            // Only save position when not maximized; the maximized position is
            // the OS-managed full-screen rect, which isn't useful to restore.
            if !maximized
                && let Some(pos) = frame.info().window_info.position {
                self.config.window_x = Some(pos.x as i32);
                self.config.window_y = Some(pos.y as i32);
            }

            // Drive continuous animation at the display refresh rate
            ctx.request_repaint();
        }

        fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
            // Save snapshot if configured to do so
            if self.config.save_state_on_exit
                && let Some(path) = self.config.save_state_path()
            {
                let snap = self.emu.take_snapshot();
                if let Ok(yaml) = serde_yaml::to_string(&snap) {
                    let _ = std::fs::write(path, yaml);
                }
            }
            self.config.save();
        }
    }

    // ── Slot helpers ──────────────────────────────────────────────────────────

    /// Rebuild the emulator's card manager from the slot configuration.
    ///
    /// Removes all existing cards then inserts new ones for each non-Empty slot
    /// according to `config.slot_cards`.  Only card types with a live
    /// implementation are actually inserted; unimplemented types are silently
    /// skipped (the slot remains empty).
    fn apply_slot_cards(emu: &mut Emulator, config: &Config) {
        use apple2_core::cards::col80::{Col80Card, Extended80ColCard};
        use apple2_core::cards::disk2::Disk2Card;
        use apple2_core::cards::hd::HdCard;
        use apple2_core::cards::mockingboard::MockingboardCard;
        use apple2_core::cards::mouse::MouseCard;
        use apple2_core::cards::phasor::PhasorCard;
        use apple2_core::cards::fourplay::FourPlayCard;
        use apple2_core::cards::noslotclock::NoSlotClockCard;
        use apple2_core::cards::ramworks::RamWorksCard;
        use apple2_core::cards::sam::SamCard;
        use apple2_core::cards::saturn::Saturn128KCard;
        use apple2_core::cards::snesmax::SnesMaxCard;
        use apple2_core::cards::ssc::SscCard;
        use apple2_core::cards::printer::PrinterCard;
        use apple2_core::cards::vidhd::VidHdCard;
        use apple2_core::cards::z80card::Z80Card;
        use apple2_core::cards::uthernet::UthernCard;
        // Clear all slots first
        for slot in 0..apple2_core::card::NUM_SLOTS {
            emu.bus.cards.remove(slot);
        }
        // Re-insert according to config
        for (slot, &card_type) in config.slot_cards.iter().enumerate() {
            match card_type {
                CardType::Disk2 => {
                    emu.bus.cards.insert(Box::new(Disk2Card::new(slot)));
                }
                CardType::GenericHdd => {
                    let mut card = HdCard::new(slot);
                    // Auto-load HDD images from config
                    if let Some(ref p) = config.last_hdd1
                        && let Ok(data) = std::fs::read(p)
                    {
                        card.load_image(0, data);
                    }
                    if let Some(ref p) = config.last_hdd2
                        && let Ok(data) = std::fs::read(p)
                    {
                        card.load_image(1, data);
                    }
                    emu.bus.cards.insert(Box::new(card));
                }
                CardType::Mockingboard => {
                    emu.bus.cards.insert(Box::new(MockingboardCard::new(slot)));
                }
                CardType::MouseInterface => {
                    emu.bus.cards.insert(Box::new(MouseCard::new(slot)));
                }
                CardType::Ssc => {
                    emu.bus.cards.insert(Box::new(SscCard::new(slot)));
                }
                CardType::Phasor => {
                    emu.bus.cards.insert(Box::new(PhasorCard::new(slot)));
                }
                CardType::Col80 => {
                    emu.bus.cards.insert(Box::new(Col80Card::new(slot)));
                }
                CardType::Extended80Col => {
                    emu.bus.cards.insert(Box::new(Extended80ColCard::new(slot)));
                }
                CardType::Sam => {
                    emu.bus.cards.insert(Box::new(SamCard::new(slot)));
                }
                CardType::GenericClock => {
                    emu.bus.cards.insert(Box::new(NoSlotClockCard::new(slot)));
                }
                CardType::FourPlay => {
                    emu.bus.cards.insert(Box::new(FourPlayCard::new(slot)));
                }
                CardType::SnesMax => {
                    emu.bus.cards.insert(Box::new(SnesMaxCard::new(slot)));
                }
                CardType::Saturn128K => {
                    emu.bus.cards.insert(Box::new(Saturn128KCard::new(slot)));
                }
                CardType::RamWorksIII => {
                    emu.bus.cards.insert(Box::new(RamWorksCard::new(slot)));
                }
                CardType::GenericPrinter => {
                    emu.bus.cards.insert(Box::new(PrinterCard::new(slot)));
                }
                CardType::VidHD => {
                    emu.bus.cards.insert(Box::new(VidHdCard::new(slot)));
                }
                CardType::Z80 => {
                    emu.bus.cards.insert(Box::new(Z80Card::new(slot)));
                }
                CardType::Uthernet => {
                    emu.bus.cards.insert(Box::new(UthernCard::new_uthernet1(slot)));
                }
                CardType::Uthernet2 => {
                    emu.bus.cards.insert(Box::new(UthernCard::new_uthernet2(slot)));
                }
                _ => {} // not yet implemented — leave slot empty
            }
        }
        // Aux slot (slot 8 / SLOT_AUX)
        match config.aux_slot_card {
            CardType::Extended80Col => {
                emu.bus.cards.insert_aux(Box::new(Extended80ColCard::new(apple2_core::card::SLOT_AUX)));
            }
            CardType::Col80 => {
                emu.bus.cards.insert_aux(Box::new(Col80Card::new(apple2_core::card::SLOT_AUX)));
            }
            CardType::RamWorksIII => {
                emu.bus.cards.insert_aux(Box::new(RamWorksCard::new(apple2_core::card::SLOT_AUX)));
            }
            _ => {} // Empty or unsupported — leave aux slot empty
        }
    }

    // ── Widget helpers ────────────────────────────────────────────────────────

    /// Toolbar button: raised Win9x bevel with a BMP icon centred inside.
    ///
    /// Falls back to a labelled button if the texture is not available.
    fn icon_btn(
        ui: &mut egui::Ui,
        tex: Option<&egui::TextureHandle>,
        fallback: &str,
        tooltip: &str,
        btn_size: Vec2,
        img_size: Vec2,
    ) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(btn_size, Sense::click());

        // Background fill
        let bg = if resp.is_pointer_button_down_on() {
            Color32::from_rgb(180, 178, 170) // slightly darker when pressed
        } else {
            WIN_FACE
        };
        ui.painter().rect_filled(rect, 2.0, bg);

        // Raised bevel (inverts when pressed)
        let (tl, br) = if resp.is_pointer_button_down_on() {
            (WIN_SHADOW, WIN_LIGHT)
        } else {
            (WIN_LIGHT, WIN_SHADOW)
        };
        let w = 1.0f32;
        let p = ui.painter();
        // top
        p.line_segment([rect.left_top(),  rect.right_top()],    Stroke::new(w, tl));
        // left
        p.line_segment([rect.left_top(),  rect.left_bottom()],  Stroke::new(w, tl));
        // bottom
        p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(w, br));
        // right
        p.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(w, br));

        if let Some(t) = tex {
            // Centre the icon image inside the button
            let offset = if resp.is_pointer_button_down_on() {
                Vec2::new(1.0, 1.0)
            } else {
                Vec2::ZERO
            };
            let img_rect = Rect::from_center_size(rect.center() + offset, img_size);
            ui.painter().image(
                t.id(),
                img_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            // Fallback: text label centred in the button
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                fallback,
                FontId::proportional(17.0),
                Color32::BLACK,
            );
        }

        if resp.hovered() {
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, WIN_DSHADOW));
        }

        resp.on_hover_text(tooltip)
    }

    /// Coloured disk-activity LED widget.
    fn disk_led(ui: &mut egui::Ui, label: &str, active: bool, writing: bool) {
        let color = if !active {
            Color32::from_rgb(25, 25, 25)
        } else if writing {
            Color32::from_rgb(210, 40, 40)
        } else {
            Color32::from_rgb(40, 200, 40)
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(26.0, 16.0), Sense::hover());
        ui.painter().rect_filled(rect, 3.0, color);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(9.0),
            Color32::WHITE,
        );
    }

    // ── Screenshot ────────────────────────────────────────────────────────────

    /// Save `pixels` (RGBA8888, row-major) as a 24bpp BMP file.
    fn save_screenshot(pixels: &[u8], w: usize, h: usize) {
        let path = screenshot_path();
        let Some(path) = path else { return };
        let row_stride = (w * 3).div_ceil(4) * 4;
        let pixel_bytes = row_stride * h;
        let file_size = (14 + 40 + pixel_bytes) as u32;

        let mut data = Vec::with_capacity(file_size as usize);

        // BMP file header (14 bytes)
        data.extend_from_slice(b"BM");
        data.extend_from_slice(&file_size.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes()); // reserved
        data.extend_from_slice(&54u32.to_le_bytes()); // pixel offset

        // DIB header / BITMAPINFOHEADER (40 bytes)
        data.extend_from_slice(&40u32.to_le_bytes());  // header size
        data.extend_from_slice(&(w as i32).to_le_bytes());
        data.extend_from_slice(&(-(h as i32)).to_le_bytes()); // negative = top-down
        data.extend_from_slice(&1u16.to_le_bytes());   // planes
        data.extend_from_slice(&24u16.to_le_bytes());  // bpp
        data.extend_from_slice(&0u32.to_le_bytes());   // compression (none)
        data.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
        data.extend_from_slice(&2835u32.to_le_bytes()); // X pixels/metre
        data.extend_from_slice(&2835u32.to_le_bytes()); // Y pixels/metre
        data.extend_from_slice(&0u32.to_le_bytes());   // colors used
        data.extend_from_slice(&0u32.to_le_bytes());   // important colors

        // Pixel data (24bpp BGR, top-down because we used negative height)
        for row in 0..h {
            let mut written = 0usize;
            for col in 0..w {
                let base = (row * w + col) * 4;
                let r = pixels[base];
                let g = pixels[base + 1];
                let b = pixels[base + 2];
                data.push(b);
                data.push(g);
                data.push(r);
                written += 3;
            }
            // Pad row to 4-byte boundary
            while !written.is_multiple_of(4) { data.push(0); written += 1; }
        }

        let _ = std::fs::write(&path, &data);
        eprintln!("Screenshot saved: {}", path.display());
    }

    /// Returns a timestamped path in %APPDATA%\applewin-rs\screenshots\ (Windows)
    /// or the current directory (other platforms).
    fn screenshot_path() -> Option<PathBuf> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let fname = format!("screenshot_{ts}.bmp");

        #[cfg(windows)]
        {
            let appdata = std::env::var_os("APPDATA")?;
            let dir = PathBuf::from(appdata).join("applewin-rs").join("screenshots");
            std::fs::create_dir_all(&dir).ok()?;
            Some(dir.join(fname))
        }
        #[cfg(not(windows))]
        {
            Some(PathBuf::from(fname))
        }
    }

    // ── File dialog ───────────────────────────────────────────────────────────

    fn open_disk_dialog(title: &str, start_dir: Option<&str>) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new()
            .set_title(title)
            .add_filter(
                "Apple II Disk Images",
                &["dsk", "do", "po", "nib", "woz", "hdv", "2mg", "img"],
            )
            .add_filter("All Files", &["*"]);
        if let Some(dir) = start_dir {
            d = d.set_directory(dir);
        }
        d.pick_file()
    }

    fn open_hdd_dialog(title: &str, start_dir: Option<&str>) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new()
            .set_title(title)
            .add_filter("Apple II HDD Images", &["hdv", "po", "2mg", "img"])
            .add_filter("All Files", &["*"]);
        if let Some(dir) = start_dir {
            d = d.set_directory(dir);
        }
        d.pick_file()
    }

    // ── 3D sunken bevel (Windows 9x "SunkenBox" look) ─────────────────────────

    fn draw_sunken_bevel(painter: &egui::Painter, outer: Rect) {
        // Outer ring: dark shadow top/left, highlight bottom/right
        bevel_ring(painter, outer,              2.0, WIN_DSHADOW, WIN_LIGHT);
        // Inner ring: mid-shadow top/left, face-colour bottom/right
        bevel_ring(painter, outer.shrink(2.0),  2.0, WIN_SHADOW,  WIN_HILIGHT);
    }

    /// Draw top+left edges in `tl` colour and bottom+right edges in `br` colour.
    fn bevel_ring(painter: &egui::Painter, r: Rect, w: f32, tl: Color32, br: Color32) {
        let h   = w / 2.0;
        let stl = Stroke::new(w, tl);
        let sbr = Stroke::new(w, br);
        // top
        painter.line_segment(
            [Pos2::new(r.left(), r.top() + h), Pos2::new(r.right(), r.top() + h)],
            stl,
        );
        // left
        painter.line_segment(
            [Pos2::new(r.left() + h, r.top()), Pos2::new(r.left() + h, r.bottom())],
            stl,
        );
        // bottom
        painter.line_segment(
            [Pos2::new(r.left(), r.bottom() - h), Pos2::new(r.right(), r.bottom() - h)],
            sbr,
        );
        // right
        painter.line_segment(
            [Pos2::new(r.right() - h, r.top()), Pos2::new(r.right() - h, r.bottom())],
            sbr,
        );
    }

    // ── Entry point ───────────────────────────────────────────────────────────

    /// Map an egui `Key` + shift state to the Apple II ASCII byte for that key.
    /// Letters are always returned as uppercase (Apple II convention).
    /// Returns `None` for keys with no printable Apple II equivalent.
    fn apple2_ascii_for_key(key: Key, shift: bool) -> Option<u8> {
        let c: u8 = match key {
            // Letters — always uppercase on Apple II
            Key::A => b'A', Key::B => b'B', Key::C => b'C', Key::D => b'D',
            Key::E => b'E', Key::F => b'F', Key::G => b'G', Key::H => b'H',
            Key::I => b'I', Key::J => b'J', Key::K => b'K', Key::L => b'L',
            Key::M => b'M', Key::N => b'N', Key::O => b'O', Key::P => b'P',
            Key::Q => b'Q', Key::R => b'R', Key::S => b'S', Key::T => b'T',
            Key::U => b'U', Key::V => b'V', Key::W => b'W', Key::X => b'X',
            Key::Y => b'Y', Key::Z => b'Z',
            // Digits and shifted symbols (standard US layout)
            Key::Num0 => if shift { b')' } else { b'0' },
            Key::Num1 => if shift { b'!' } else { b'1' },
            Key::Num2 => if shift { b'@' } else { b'2' },
            Key::Num3 => if shift { b'#' } else { b'3' },
            Key::Num4 => if shift { b'$' } else { b'4' },
            Key::Num5 => if shift { b'%' } else { b'5' },
            Key::Num6 => if shift { b'^' } else { b'6' },
            Key::Num7 => if shift { b'&' } else { b'7' },
            Key::Num8 => if shift { b'*' } else { b'8' },
            Key::Num9 => if shift { b'(' } else { b'9' },
            Key::Space => b' ',
            _ => return None,
        };
        Some(c)
    }

    pub fn run(emu: Emulator, config: Config) {
        // Always open at 2× scale on startup, matching AppleWin's
        // kDEFAULT_VIEWPORT_SCALE = 2.  AppleWin does not restore a
        // saved scale; it always starts at 2× (falling back to 1× only
        // if the monitor is too small).
        //
        // Width overhead:  BEVEL×2 (8) + BTN_PANEL_W (55) + inner margins (16)
        //                = 79 px  →  round up to 80 for a clean number.
        // Height overhead: BEVEL×2 (8) + menu-bar (≈26) + status-bar (≈26)
        //                + inner margins (16) = 76 px  →  round up to 80.
        // Both match AppleWin's VIEWPORTX×2+BUTTONCX (55) plus our extra chrome.
        const W_OVERHEAD: f32 = 80.0;
        const H_OVERHEAD: f32 = 80.0;

        let win_w = SCREEN_W as f32 * 2.0 + W_OVERHEAD;
        let win_h = SCREEN_H as f32 * 2.0 + H_OVERHEAD;

        // When the window was last closed maximized, start maximized again and
        // skip the saved pos/size (they're irrelevant in maximized state).
        // We disable eframe's own persist_window so it doesn't fight our config.
        let (initial_size, initial_pos) = if config.window_maximized {
            (None, None)
        } else {
            let pos = match (config.window_x, config.window_y) {
                (Some(x), Some(y)) => Some(egui::Pos2::new(x as f32, y as f32)),
                _ => None,
            };
            (Some(egui::vec2(win_w, win_h)), pos)
        };

        let options = eframe::NativeOptions {
            initial_window_size: initial_size,
            initial_window_pos:  initial_pos,
            maximized:           config.window_maximized,
            min_window_size: Some(egui::vec2(
                SCREEN_W as f32 + W_OVERHEAD,
                SCREEN_H as f32 + H_OVERHEAD,
            )),
            // Disable eframe's built-in window-state persistence — we handle
            // position, size, and maximized state ourselves via config.toml.
            // Leaving persist_window: true (the default) causes eframe to save
            // a fixed pixel size and restore it via set_inner_size() on startup,
            // which fights our own initial_window_size and prevents the window
            // from starting in the correct maximized state.
            persist_window: false,
            ..Default::default()
        };
        eframe::run_native(
            "AppleWin-rs",
            options,
            Box::new(move |_cc| Box::new(EmulatorApp::new(emu, config))),
        )
        .expect("eframe failed");
    }

    // ── Apple IIe character ROM ────────────────────────────────────────────────
    //
    // 4 KB video ROM: glyphs 0x00-0x3F live at 0x0400, glyphs 0x40-0x7F at 0x0600.
    // Each glyph is 8 consecutive bytes.  Raw bytes are stored inverted (0=lit).
    // Per UTAIIe §8-30, the ROM bit order is also reversed: bit 0 = leftmost pixel.
    // We XOR-invert then bit-reverse bits [6:0] and shift left by 1 to produce our
    // MSB-first format (bit 7 = leftmost pixel).
    static VIDEO_ROM: &[u8] = include_bytes!("../roms/Apple2e_Enhanced_Video.rom");

    fn build_font_from_rom(rom: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(128 * 8);
        for idx in 0u8..128 {
            let base = if idx < 64 {
                0x0400 + (idx as usize) * 8
            } else {
                0x0600 + (idx as usize - 64) * 8
            };
            for row in 0..8 {
                let inv = rom[base + row] ^ 0xFF; // invert polarity: 1 = lit pixel
                // Bit-reverse bits [6:0]: ROM bit 0 = leftmost → our bit 7 = leftmost
                let mut d: u8 = 0;
                let mut n = inv;
                for _ in 0..7 {
                    d = (d << 1) | (n & 1);
                    n >>= 1;
                }
                out.push(d << 1); // shift into bits [7:1]; bit 7 = leftmost pixel
            }
        }
        out
    }
}

