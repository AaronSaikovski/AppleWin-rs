//! GUI front end (eframe 0.30 + egui 0.30).
//!
//! `mod.rs` owns the shared state (`EmulatorApp`), constants, and the
//! eframe `update()` loop; the per-frame work is implemented as
//! `EmulatorApp` methods in the sibling modules:
//!
//! - [`emulation`]      — execution pacing, reset, disk loading, slot cards
//! - [`audio`]          — cpal stream, ring buffer, per-frame sample synthesis
//! - [`input`]          — keyboard events, clipboard paste, global shortcuts
//! - [`joystick`]       — gamepad/keyboard/mouse joystick & paddle emulation
//! - [`render`]         — framebuffer rendering, texture upload, screenshots
//! - [`menu`]           — the top menu bar (File/Machine/View/Help)
//! - [`statusbar`]      — the bottom status bar (drive LEDs, hints)
//! - [`toolbar`]        — the right-hand icon button strip
//! - [`dialogs`]        — modal dialogs (reboot/about) and file pickers
//! - [`debugger_panel`] — the debugger command bar and shortcuts
//! - [`screen`]         — the central Apple II screen panel + drag-and-drop
//! - [`actions`]        — the `DeferredActions` request struct and its application
//! - [`settings`]       — the Settings dialog shell + per-card options popups
//! - [`settings_tabs`]  — the individual Settings tabs (Machine/Video/…)
//! - [`widgets`]        — BMP icon assets and small chrome widgets
//! - [`window`]         — native-window / viewport helpers (size, position, fullscreen)

mod actions;
mod audio;
mod debugger_panel;
mod dialogs;
mod emulation;
mod input;
mod joystick;
mod menu;
mod render;
mod screen;
mod settings;
mod settings_tabs;
mod statusbar;
mod toolbar;
mod widgets;
mod window;

#[allow(unused_imports)]
use {
    actions::*, audio::*, debugger_panel::*, dialogs::*, emulation::*, input::*, joystick::*,
    menu::*, render::*, screen::*, settings::*, settings_tabs::*, statusbar::*, toolbar::*,
    widgets::*, window::*,
};

use apple2_core::emulator::Emulator;
use apple2_video::{
    framebuffer::Framebuffer,
    ntsc::{CharRom, NtscRenderer},
};
use eframe::egui::{
    self, Align, Color32, ColorImage, FontId, Key, Layout, Pos2, Rect, RichText, Sense, Stroke,
    TextureOptions, Vec2,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::EmuCore;
use crate::config::{
    ALL_JOYSTICK_TYPES, ALL_VIDEO_TYPES, Config, IMPLEMENTED_CARDS, VideoType, card_name, cpu_name,
    joystick_type_name, model_name, video_type_name,
};
use apple2_core::{
    card::CardType,
    model::{Apple2Model, CpuType},
};

// ── Dimensions ───────────────────────────────────────────────────────────

const SCREEN_W: usize = 560;
const SCREEN_H: usize = 384;

/// Precomputed source-column table mapping each dst x (0..560) to a src x
/// in the 640-wide IIgs SHR buffer. Avoids per-pixel division/modulo in the
/// scaling hot loop (≈215 K pixels per frame).
const SHR_SRC_X: [u16; SCREEN_W] = {
    let mut t = [0u16; SCREEN_W];
    let mut i = 0;
    while i < SCREEN_W {
        let v = i * 640 / SCREEN_W;
        t[i] = if v >= 640 { 639 } else { v as u16 };
        i += 1;
    }
    t
};

/// Precomputed source-row table mapping each dst y (0..384) to a src y
/// in the 400-tall IIgs SHR buffer.
const SHR_SRC_Y: [u16; SCREEN_H] = {
    let mut t = [0u16; SCREEN_H];
    let mut i = 0;
    while i < SCREEN_H {
        let v = i * 400 / SCREEN_H;
        t[i] = if v >= 400 { 399 } else { v as u16 };
        i += 1;
    }
    t
};
/// 3D bevel around the Apple II screen (2 layers × 2 px each)
const BEVEL: f32 = 4.0;
/// Width of the right-side button strip
const BTN_PANEL_W: f32 = 55.0;

// ── Windows 9x-style palette for chrome ──────────────────────────────────

const WIN_FACE: Color32 = Color32::from_rgb(212, 208, 200);
const WIN_LIGHT: Color32 = Color32::from_rgb(255, 255, 255);
const WIN_HILIGHT: Color32 = Color32::from_rgb(212, 208, 200);
const WIN_SHADOW: Color32 = Color32::from_rgb(128, 128, 128);
const WIN_DSHADOW: Color32 = Color32::from_rgb(64, 64, 64);
/// All 8 toolbar icon textures, pre-loaded at startup.
struct Icons {
    help: Option<egui::TextureHandle>,
    run: Option<egui::TextureHandle>,
    d1: Option<egui::TextureHandle>,
    d2: Option<egui::TextureHandle>,
    swap: Option<egui::TextureHandle>,
    full: Option<egui::TextureHandle>,
    debug: Option<egui::TextureHandle>,
    setup: Option<egui::TextureHandle>,
}

impl Icons {
    fn load(ctx: &egui::Context) -> Self {
        let opts = TextureOptions::NEAREST;
        let load = |name: &str, raw: &[u8]| -> Option<egui::TextureHandle> {
            let (w, h, rgba) = decode_bmp_rgba(raw)?;
            let img = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            Some(ctx.load_texture(name, img, opts))
        };
        Self {
            help: load("icon_help", BMP_HELP),
            run: load("icon_run", BMP_RUN),
            d1: load("icon_d1", BMP_D1),
            d2: load("icon_d2", BMP_D2),
            swap: load("icon_swap", BMP_SWAP),
            full: load("icon_full", BMP_FULL),
            debug: load("icon_debug", BMP_DEBUG),
            setup: load("icon_setup", BMP_SETUP),
        }
    }
}

// ── App state ─────────────────────────────────────────────────────────────

struct EmulatorApp {
    emu: Emulator,
    /// Optional IIgs emulator — when `Some`, execution and rendering use
    /// this instead of the IIe `emu` field.
    iigs: Option<apple2_iigs::emulator::IIgsEmulator>,
    renderer: NtscRenderer,
    fb: Framebuffer,
    texture: Option<egui::TextureHandle>,
    logo_texture: Option<egui::TextureHandle>,
    icons: Option<Icons>,
    frame_no: u32,
    disk1: Option<PathBuf>,
    disk2: Option<PathBuf>,
    fullscreen: bool,
    show_about: bool,
    // Configuration
    config: Config,
    show_settings: bool,
    pending_config: Config,
    settings_tab: usize,
    /// Which slot the Disk II card is currently installed in (derived from config).
    disk_slot: usize,
    /// Pending reset type: Some(true)=hard-reset, Some(false)=soft-reset,
    /// None=no pending.  Set when confirm_reboot is true.
    pending_reset: Option<bool>,
    // Audio
    audio_buf: Option<AudioBuf>,
    _audio_stream: Option<cpal::Stream>,
    speaker_state: bool,
    last_audio_cycle: u64,
    audio_sample_rate: u32,
    /// Fractional CPU cycles that didn't make a full sample last frame.
    spkr_cycle_rem: f64,
    /// DC-filter counter (matches Windows `g_uDCFilterState`).
    /// Reset to 32768+10000 on every $C030 toggle; linearly fades to 0.
    dc_filter_ctr: u32,
    /// Wall-clock timestamp of the previous update() call.
    /// Used to execute exactly the right number of CPU cycles regardless of
    /// how often egui calls update() (window resize can cause burst repaints).
    last_frame_time: std::time::Instant,
    /// Characters queued for paste injection into the Apple II keyboard.
    paste_buf: std::collections::VecDeque<u8>,
    // Debugger
    show_debugger: bool,
    debugger: apple2_debugger::DebuggerState,
    debugger_cmd_input: String,
    #[allow(dead_code)]
    debugger_bp_input: String,
    // Per-slot options popup open flags (one per slot, 0..8)
    slot_options_open: [bool; 8],
    // RGB video renderer (used when VideoType::ColorRGB is selected)
    rgb_renderer: apple2_video::rgb::RgbRenderer,
    // WAV audio recording
    wav_recorder: Option<apple2_audio::wav_writer::WavRecorder>,
    // Status message (shown briefly after save/load state)
    status_msg: Option<String>,
    // Gamepad input (gilrs)
    gilrs: Option<gilrs::Gilrs>,
    /// The active gamepad ID (first connected pad, or None).
    active_gamepad: Option<gilrs::GamepadId>,
    /// Reusable 640x400 scratch buffer for SHR rendering (IIgs).
    /// Allocated once to avoid per-frame 1MB allocations.
    shr_scratch: Vec<u32>,
    /// Reusable scratch for WAV recorder sample chunks.
    wav_scratch: Vec<f32>,
    /// Reusable scratch for draining speaker_toggles without dropping
    /// the Vec's preallocated capacity every frame.
    speaker_toggles_scratch: Vec<u64>,
    /// Reusable scratch for speaker PCM samples — synthesized without the
    /// ring-buffer mutex held, then bulk-pushed under a single lock.
    speaker_scratch: Vec<f32>,
    /// Reusable scratch for Ensoniq DOC PCM samples (IIgs).
    ensoniq_scratch: Vec<f32>,
}

impl EmulatorApp {
    fn new_with_core(core: EmuCore, config: Config) -> Self {
        let (emu, iigs) = match core {
            EmuCore::IIe(e) => (e, None),
            EmuCore::IIgs(g) => {
                // Create a dummy IIe emulator for the GUI framework (rendering, etc.)
                // The actual execution goes through the IIgs emulator.
                let dummy = Emulator::new(
                    crate::APPLE2E_ROM.to_vec(),
                    Apple2Model::AppleIIgs,
                    CpuType::Cpu65C02,
                );
                (dummy, Some(*g))
            }
        };
        Self::new_inner(emu, iigs, config)
    }

    fn new_inner(
        mut emu: Emulator,
        mut iigs: Option<apple2_iigs::emulator::IIgsEmulator>,
        config: Config,
    ) -> Self {
        // Install cards according to slot configuration (IIe only)
        if iigs.is_none() {
            apply_slot_cards(&mut emu, &config);
        }

        // Build CharRom from the embedded Apple IIe video ROM
        let font_data = build_font_from_rom(VIDEO_ROM);
        let char_rom = CharRom::new(font_data);
        let tv_mode = matches!(
            config.video_type,
            crate::config::VideoType::ColorTV | crate::config::VideoType::MonoTV
        );
        let mut renderer = NtscRenderer::new(char_rom.clone(), config.scanlines, tv_mode);
        renderer.mono_tint = config.mono_tint();
        renderer.color_vertical_blend = config.color_vertical_blend;
        let rgb_renderer = apple2_video::rgb::RgbRenderer::new(char_rom, config.scanlines);

        // Initialise audio output (best-effort; silent if unavailable)
        let (audio_buf, _audio_stream, audio_sample_rate) = match init_audio() {
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
            if let Some(ref mut iigs_emu) = iigs {
                if Self::load_iigs_disk(iigs_emu, 0, &path) {
                    disk1 = Some(path);
                }
            } else if let Ok(data) = std::fs::read(&path) {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                emu.bus.load_disk(disk_slot, 0, &data, &ext);
                emu.bus.set_disk_path(disk_slot, 0, path.clone());
                disk1 = Some(path);
            }
        }
        if let Some(ref p) = config.last_disk2 {
            let path = PathBuf::from(p);
            if let Some(ref mut iigs_emu) = iigs {
                if Self::load_iigs_disk(iigs_emu, 1, &path) {
                    disk2 = Some(path);
                }
            } else if let Ok(data) = std::fs::read(&path) {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
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
            && let Ok(snap) = serde_yaml::from_str::<apple2_core::emulator::EmulatorSnapshot>(&yaml)
        {
            emu.restore_snapshot(&snap);
            emu.mode = apple2_core::emulator::AppMode::Running;
        }

        // Initialise gamepad input (best-effort; disabled if unavailable).
        let (gilrs_inst, active_gamepad) = match gilrs::Gilrs::new() {
            Ok(g) => {
                // Pick the first connected gamepad, if any.
                let id = g.gamepads().next().map(|(id, _)| id);
                if let Some(id) = id
                    && let Some(gp) = g.connected_gamepad(id)
                {
                    println!("Gamepad detected: {}", gp.name());
                }
                (Some(g), id)
            }
            Err(e) => {
                eprintln!("Warning: gamepad input unavailable: {e}");
                (None, None)
            }
        };

        let initial_cycle = emu.cpu.cycles;
        let pending_config = config.clone();

        // If IIgs mode, go straight to Running (no logo screen)
        if iigs.is_some() {
            emu.mode = apple2_core::emulator::AppMode::Running;
        }

        Self {
            emu,
            iigs,
            renderer,
            fb: Framebuffer::new(),
            texture: None,
            logo_texture: None,
            icons: None,
            frame_no: 0,
            disk1,
            disk2,
            fullscreen: false,
            show_about: false,
            config,
            show_settings: false,
            pending_config,
            settings_tab: 0,
            disk_slot,
            pending_reset: None,
            audio_buf,
            _audio_stream,
            speaker_state: false,
            last_audio_cycle: initial_cycle,
            audio_sample_rate,
            spkr_cycle_rem: 0.0,
            dc_filter_ctr: 0,
            last_frame_time: std::time::Instant::now(),
            paste_buf: std::collections::VecDeque::new(),
            show_debugger: false,
            debugger: {
                let mut d = apple2_debugger::DebuggerState::new();
                d.load_apple2_symbols();
                d
            },
            debugger_cmd_input: String::new(),
            debugger_bp_input: String::new(),
            slot_options_open: [false; 8],
            rgb_renderer,
            wav_recorder: None,
            status_msg: None,
            gilrs: gilrs_inst,
            active_gamepad,
            shr_scratch: vec![0u32; 640 * 400],
            wav_scratch: Vec::with_capacity(1024),
            speaker_toggles_scratch: Vec::with_capacity(65536),
            speaker_scratch: Vec::with_capacity(2048),
            ensoniq_scratch: Vec::with_capacity(2048),
        }
    }
}

// ── eframe App impl ───────────────────────────────────────────────────────

impl eframe::App for EmulatorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_no = self.frame_no.wrapping_add(1);

        // Load icons and logo texture on first frame
        if self.icons.is_none() {
            self.icons = Some(Icons::load(ctx));
        }
        if self.logo_texture.is_none()
            && let Some((w, h, rgba)) = decode_bmp24_rgba(BMP_LOGO)
        {
            let img = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
            self.logo_texture = Some(ctx.load_texture("logo", img, TextureOptions::LINEAR));
        }

        let in_logo_mode = self.emu.mode == apple2_core::emulator::AppMode::Logo;

        self.run_emulation(in_logo_mode);

        self.synth_speaker_audio();
        self.synth_ensoniq_audio();
        self.synth_mockingboard_audio();
        self.tap_wav_recording();

        if self.handle_input(ctx, in_logo_mode) {
            request_close(ctx);
            return;
        }

        let tex_id = self.upload_frame_texture(ctx, in_logo_mode);

        // ── UI panels (deferred actions applied after layout) ─────────────
        let mut act = DeferredActions::default();
        self.show_menu_bar(ctx, &mut act);
        self.show_status_bar(ctx, in_logo_mode);
        self.show_button_strip(ctx, &mut act);
        self.show_reboot_dialog(ctx);
        self.show_debugger_panel(ctx);
        self.show_settings_dialog(ctx);
        self.show_card_popups(ctx);
        self.show_about_dialog(ctx);
        self.show_central_panel(ctx, in_logo_mode, tex_id);
        self.apply_deferred_actions(ctx, act, in_logo_mode);
        self.handle_drag_and_drop(ctx);

        // F11 fullscreen shortcut (supplement to the action already handled above)
        if ctx.input(|i| i.key_pressed(Key::F11)) {
            self.toggle_fullscreen(ctx);
        }

        // Persist window size/position so the next launch restores it.
        self.track_window_state(ctx);

        // Drive continuous animation at the display refresh rate — but skip
        // when the debugger has halted execution (Stepping). In that state
        // nothing is advancing on-screen, so egui can rely on input-driven
        // repaints, saving CPU while the user inspects state.
        let needs_repaint = self.emu.mode != apple2_core::emulator::AppMode::Stepping
            || self.show_settings
            || self.show_about
            || self.show_debugger;
        if needs_repaint {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Save snapshot if configured to do so (not available for IIgs yet)
        if self.config.save_state_on_exit
            && self.iigs.is_none()
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

pub fn run_with_core(core: EmuCore, config: Config) {
    let options = eframe::NativeOptions {
        viewport: build_viewport(&config),
        persist_window: false,
        ..Default::default()
    };

    let title = match &core {
        EmuCore::IIgs(_) => "AppleWin-rs — Apple IIgs",
        EmuCore::IIe(_) => "AppleWin-rs",
    };

    eframe::run_native(
        title,
        options,
        Box::new(move |_cc| Ok(Box::new(EmulatorApp::new_with_core(core, config)))),
    )
    .expect("eframe failed");
}
