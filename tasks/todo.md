# Performance & Maintainability Pass (approved plan)

Plan file: ~/.claude/plans/can-you-review-the-splendid-beaver.md

## Phase 1 — Quick wins (one commit) ✅ committed c640463
- [x] `[profile.release]` lto + codegen-units=1 in workspace Cargo.toml
- [x] `#[inline]` on Bus::flag_byte / update_irq_line / process_card_dma (bus.rs)
- [x] `#[inline]` on CardManager::any_irq_active (card.rs)
- [x] `#[inline]` on text_row_offset / hgr_row_offset (ntsc.rs)
- [x] Remove SSI-263 no-op render loop (ssi263.rs)
- [x] Vec::with_capacity for Mega II speaker_toggles (mega2.rs)
- [x] Remove per-frame clones: recent_disks/recent_hdds menus, debugger_cmd_input (main.rs)
- [x] Four checks + headless build (3 pre-existing headless dead-code warnings noted; fix in 2b); commit

## Phase 2a — Extract update() into methods ✅ committed 46927cc
- [x] DeferredActions struct replacing act_* locals
- [x] Extract 19 section methods in place; update() shrinks to ~60 lines
- [x] Four checks + headless build; commit

## Phase 2b — Split mod gui into files ✅ committed 9272138
- [x] gui/{mod,emulation,audio,input,render,panels,settings,widgets}.rs
- [x] Four checks + headless build (now warning-free); commit
- [ ] Manual GUI smoke test — NOT RUN (tool permission classifier outage);
      user should launch the app once: boot, keys, menus, settings, debugger

## Phase 3 — bus.rs soft-switch extraction ✅ committed b2af658
- [x] bus.rs → bus/mod.rs (994) + bus/soft_switches.rs (499)
- [x] Four checks + headless build; iic_boot_trace passes; commit

## CHANGELOG
- [x] Entries under [Unreleased]: Changed (module splits) + Performance

## Review

All three phases landed on `fixes/v1.1.6` as five commits (incl. the
pre-existing Airheart fix committed first as f9ccedb):

- **Phase 1 (c640463)** — `[profile.release]` fat LTO + codegen-units=1;
  `#[inline]` on Bus::flag_byte/update_irq_line/process_card_dma,
  CardManager::any_irq_active, text_row_offset, hgr_row_offset; removed
  per-frame clones (recent-disk/HDD menus, debugger cmd input); deleted
  the SSI-263 no-op render loop; preallocated Mega II speaker_toggles.
- **Phase 2a (46927cc)** — update() (2,770 lines) decomposed into 19
  `EmulatorApp` methods + `DeferredActions` struct, in place, pure motion.
- **Phase 2b (9272138)** — `mod gui` moved out of main.rs into 8 files
  under `gui/`; main.rs 4,431 → 271 lines; cross-module items pub(super);
  include_bytes! paths adjusted; headless dead-code warnings fixed via
  `#[cfg(feature = "gui")]`.
- **Phase 3 (b2af658)** — soft-switch dispatch (492 lines) extracted to
  `bus/soft_switches.rs`; bus/mod.rs at 994 lines.

Verification on every phase: `cargo fmt --all`, `clippy --workspace
--all-targets -- -D warnings` (clean), `cargo build --release`,
`cargo test` (15 suites, 486 tests, 0 failures), plus the headless build.
Outstanding: one manual GUI launch (blocked by a tooling outage, not by
the code — automated checks all green).

---
---

# ARCHIVE — Fix Apple //c VBL semantics (startup screech in some games) [DONE, committed f9ccedb]

## Problem
Some IIc-aware games screech on startup. They sync sound/music to vertical
blanking by polling `$C019` and acknowledging via `$C070`. On the //c,
`$C019` is a latched VBL interrupt-pending flag (bit 7 set at each VBL start,
held until an access to `$C070` clears it), and `$C05A`/`$C05B` are
DISVBL/ENVBL. The bus currently implements IIe semantics for all models
(live active-low VBL bar), so IIc-style frame-wait loops exit immediately
and speaker routines free-run at kHz rates → screech.

## Plan
- [x] Diagnose root cause (IIc VBL latch vs IIe live VBL bar)
- [x] Add //c VBL state to `Bus`: `vbl_flag`, `vbl_irq_enabled`, `next_vbl_cycle`
- [x] Hoist frame-timing constants (17030 / 12480) to module level
- [x] `$C019` read: on //c return latched flag; IIe path unchanged
- [x] `$C070` read+write: on //c also clear VBL flag (keep paddle strobe)
- [x] `$C05A`/`$C05B` read+write: on //c = DISVBL/ENVBL (not annunciator 1)
- [x] Assert IRQ line while `vbl_irq_enabled && vbl_flag` (MAME-compatible:
      flag latches regardless of enable; enable gates only the IRQ)
- [x] Drive `vbl_tick` from `Emulator::execute` loop + `step()`; reset/restore
      re-seed via `Bus::reset_vbl`
- [x] Unit tests: //c latch/ack/IRQ semantics + IIe behaviour unchanged
- [x] cargo fmt / clippy -D warnings / build --release / test
- [x] CHANGELOG.md entry

## Review

Implemented in `apple2-core`:

- `bus.rs`: new `vbl_flag` / `vbl_irq_enabled` / `next_vbl_cycle` fields;
  `vbl_tick()` latches the flag at each VBL boundary with missed-frame catch-up
  (full-speed disk bursts, debugger pauses); `reset_vbl()` re-seeds phase on
  reset/snapshot-restore. `$C019` returns the latched flag on the //c (IIe path
  untouched). `$C070` (read and write) acks the flag. `$C05A`/`$C05B` are
  DISVBL/ENVBL on the //c, annunciator 1 elsewhere. `update_irq_line()` ORs in
  `vbl_irq_enabled && vbl_flag`.
- `emulator.rs`: execute loop and `step()` check `cycles >= next_vbl_cycle`
  (a single always-false u64 compare on non-//c models, where the field is
  `u64::MAX`); `finish_reset()` and `restore_snapshot()` call `reset_vbl()`.
- Tests: 6 bus unit tests (latch/ack, schedule catch-up, ENVBL IRQ gating,
  annunciator isolation, IIe regression, reset re-seed) plus one end-to-end
  test with the real 3.5 ROM in `tests/iic_boot_trace.rs`.
- All checks green: fmt, clippy `-D warnings`, release build, 486 tests.

Known remaining //c gaps (not sound-related, left for future work):
- `$C058`/`$C059` (DISXY/ENBXY) and `$C05C`/`$C05D` (X0EDGE/Y0EDGE) mouse
  interrupt switches still act as annunciators 0/2; the //c mouse is
  approximated by a MouseCard in slot 4.
- `$C060` reads cassette input rather than the //c 40/80-column switch.
- `$C048–$C04F` (//c mouse interrupt clear strobes) are unmapped.

---

# ARCHIVE — Follow-up: screech persisted → real cause was speaker synthesis aliasing [DONE, committed f9ccedb]

## Findings (reproduced with the user's Airheart.dsk, headless probe)
- Boot beep renders correctly (546-cycle toggle intervals ≈ 937 Hz).
- Airheart start sound: ~30k toggles over ~2 s with median interval 16–22
  CPU cycles, minimum 4 — a PWM/duty-cycle technique whose carrier is above
  the 44.1 kHz output rate (~23.2 cycles/sample).
- GUI synthesis (applewin/src/main.rs) sampled only the cone state after the
  last toggle per sample → the ultrasonic carrier aliased into the audible
  band (verified offline: naive render shows full-amplitude ~3 kHz content
  where the averaged signal has zero crossings at all).

## Fix
- [x] Duty-cycle averaging in the speaker synthesis loop (time-weighted level
      across all toggles inside each sample), matching apple2-audio's
      `Speaker::render`.
- [x] `clks_per_sample`: drop `floor()` (was over-producing samples ~0.9%,
      slowly filling the ring buffer to its 2 s cap).
- [x] Preserve speaker-state parity for toggles outside the sample grid and
      when a frame renders zero samples.
- [x] fmt / clippy -D warnings / build --release / test — all green.
- [x] CHANGELOG.md entry; lesson recorded in tasks/lessons.md.
