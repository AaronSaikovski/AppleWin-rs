# Apple IIgs boot bring-up (approved)

Investigation found the IIgs crate is ~90% built and wired end-to-end, but the
firmware does not boot. A real-ROM boot trace pinned three concrete blockers.
This task fixes the first two (the ones that get the firmware visibly running).

## Evidence (boot trace, ROM 00 / ROM 01)
- ROM 01 dumps (`342-0077-B`) have their two 64KB halves swapped → reset vector
  at `$00/FFFC` reads `$0000` → CPU dead-loops at `$00/0000`.
- With a correct ROM, cold-start runs into native mode then hangs at `$FF/B53A`
  polling `LDA $C034 / BMI` with **DBR=$E1**. Banks `$E0/$E1` route straight to
  fast RAM with no `$C000-$CFFF` I/O aperture, so the read returns RAM garbage
  (`$A0`, bit 7 set) and the poll never completes.

## Blocker 1 — ROM image layout normalization  (memory.rs)  ✅
- [x] Add `normalize_rom_layout`: if the reset vector at bank `$FF/$FFFC` is
      `$0000` but the other half has a valid vector, swap the 64KB halves.
- [x] Run version detection after normalization (size-based mapping unaffected).
- [x] Verify: `IIgsMemory::new` with the ROM 01 file yields reset vector `$FA62`.

## Blocker 2 — `$E0`/`$E1` I/O aperture  (bus.rs)  ✅
- [x] Extract shared `io_read_reg` / `io_write_reg` from the slow-bank handlers.
- [x] Add `read_fast_bank` / `write_fast_bank` for `$E0`/`$E1`: decode
      `$C000-$C0FF` (I/O), `$C100-$CFFF` (slot ROM), `$D000-$FFFF` (language card
      against fast RAM); everything else → fast RAM.
- [x] Point `bank_read`/`bank_write` `$E0|$E1` arms at the new handlers.
- [x] Route `read_rom_bank` I/O through the shared helper too (correctness).

## Verification  ✅
- [x] Boot-trace probe: ROM 01 & ROM 00 now reset to `$FA62`, enter native mode,
      execute 24k+ distinct PCs (no longer a <=2-PC dead loop), and write the
      self-test pattern to the screen. Probe deleted.
- [x] Regression tests added (`swapped_rom_halves_are_normalized`,
      `canonical_rom_layout_is_not_swapped`, `fast_bank_has_io_aperture`).
- [x] `cargo fmt --all`
- [x] `cargo clippy --workspace --all-targets -- -D warnings` (clean)
- [x] `cargo build --release`
- [x] `cargo test` (all suites pass)
- [x] CHANGELOG.md entry under [Unreleased].

## Blocker 3 — Mega II / VGC interrupt system  (mega2.rs / bus.rs)  ✅
- [x] Model the interrupt registers with real hardware semantics: `$C023`
      (VGCINT status/enable), `$C032` (VGCINT clear), `$C041` (INTEN VBL/¼-sec),
      `$C046` (INTFLAG + mouse latch), `$C047` (INTCLEAR). Semantics ported from
      GSplus `moremem.c` / `sim65816.c`.
- [x] Add `Mega2::heartbeat_vbl`: raises VBL, ¼-sec (every 16 VBLs), 1-sec
      (every 60 VBLs), and VGC scan-line sources, each behind its enable gate.
      Track pending sources in `Mega2::irq_pending`.
- [x] `IIgsBus::update_interrupts` drives one heartbeat per elapsed 60 Hz frame
      (`CYCLES_PER_FRAME`), scans the SHR SCBs for scan-line-int requests, and
      composes `irq_line` from the Mega II/VGC sources + ADB keyboard.
- [x] Verify: real ROM 01 reaches interrupt-init, enables the 1-sec interrupt
      (`$C023=$05`), and the IRQ line asserts/services. Probe deleted.
- [x] Regression tests: `vbl_interrupt_fires_and_clears`,
      `quarter_second_interrupt_fires_every_16_frames`,
      `one_second_interrupt_fires_after_60_frames`, `disabled_interrupts_stay_quiet`.
- [x] fmt / clippy `-D warnings` / release build / full test suite all green.

## Blocker 4 — Language-card read-source inversion  (mega2.rs)  ✅
- [x] Root-caused "GS/OS / PRODOS 16 REQUIRES APPLE IIGS HARDWARE": the loaders
      run `LDA $C082 : SEC : JSR $FE1F : BCC ok` (select read-ROM, then call the
      ROM identity routine which does `CLC`). The `$C08x` handler had modes 0/2
      inverted, so `$C082` selected read-RAM and `$FE1F` executed uninitialised
      LC RAM → carry stayed set → loaders aborted.
- [x] Fixed `handle_language_card` to match the Apple IIe core: `HIGHRAM`
      (read RAM) set for `$C080`/`$C083`, clear for `$C081`/`$C082`.
- [x] Verified: `LDA $C082 : SEC : JSR $FE1F` on the real ROM 01/03 identity
      routine now returns carry-clear (passes the IIgs check). ROM 01 and ROM 03
      `$FE1F` are byte-identical. Probes deleted.
- [x] Regression tests: `language_card_read_source_switches`,
      `language_card_highram_flag_matches_hardware`.
- [x] fmt / clippy / release / full suite green.

Note: the GUI auto-loads ROM 03 (256KB Tenspeed v25) by preference; the fix is
ROM-independent (same `$FE1F` routine, same `$C08x` semantics). Headless full-boot
repro of ROM 03 is still incomplete (separate boot-path gaps), so verify in-GUI.

## Blocker 5 — STATEREG + ROM-bank I/O aperture (ROM 03 boot)  ✅
- [x] STATEREG `$C068` bit layout fixed: `[3]RDROM [2]LCBANK2 [1]ROMBANK
      [0]INTCXROM` (bit 3 = inverse of HIGHRAM). ROM 03 reset does
      `LDA #$0C : STA $C068` (read ROM, bank 2); the old mapping flipped it to
      read-RAM → next ROM fetch = uninitialised RAM (BRK) → garbage screen.
- [x] ROM banks `$FC-$FF` read pure ROM at `$C000-$CFFF` (no I/O aperture / slot
      cache overlay). Firmware runs real code at `$FF/$C0xx` (`JSR $C085`) and
      reaches I/O via long addressing to `$E0`/`$E1`/`$00`. Matches GSplus.
- [x] Traced ROM 03 from reset: now runs the full self-test/init (no crash);
      previously derailed at the STATEREG write and then at `JSR $C085`.
- [x] Regression tests: `statereg_read_rom_bit`, `rom_bank_c0xx_reads_rom_not_io`.
- [x] fmt / clippy / release / full suite green.

## Blocker 6 — IWM self-test + $C07x vectors + ADB GLU (firmware boots!)  ✅
- [x] `$C071-$C07F` reads ROM bank $FF (native interrupt-vector dispatch: IRQ
      vector → `$C074 = CLV; JML $E10010`). Was returning I/O → BRK loop on the
      first interrupt.
- [x] Minimal IWM (`iwm.rs`): mode/status/handshake registers so the POST IWM
      self-test and 5.25" drive probe complete (report "no disk").
- [x] Rewrote the ADB GLU command set to match the real micro-controller
      (correct command numbers, parameter lengths incl. 4/8-byte Sync, and
      responses for GetVersion/ReadConfig/ReadCharSets/ReadKbdLayouts). Fixes
      the "Fatal system error $0911" death; removed the wrong ADB-BRAM shortcut.
- [x] Result: **ROM 01 and ROM 03 both boot the firmware to the Apple IIgs
      banner** ("ROM Version 01 / 03"), then drop to the Monitor (`BRK $0003`)
      because no boot device is loaded yet.
- [x] GUI now prefers ROM 01 (342-0077-B), the standard/most-compatible image.
- [x] Regression tests: `c07x_vector_area_reads_rom`, `iwm_mode_register_selftest`,
      `adb_glu_commands_respond`. fmt/clippy/release/tests green.

## Blocker 7 — SmartPort boot: GS/OS boots! 🎉  ✅
- [x] Fixed `.2mg` parser: data offset at header `$18`, length at `$1C` (was
      reading `$08`/`$0C` → 0-block disk). This was why nothing loaded.
- [x] Added a real boot loader to the slot-5 stub: `JMP $C500` → ID bytes →
      `WDM $FD` boot trap (reads block 0 → `$0800`) → `JMP $0801`.
- [x] Added a ProDOS 8 block-driver entry ($C523, `$42-$47` convention) and put
      the SmartPort entry at ProDOS + 3 = $C526 (SmartPort ERS), fixing the P16
      loader's `JSR $C526` crash.
- [x] **Result: firmware → ProDOS 16 v1.5 loader → GS/OS "Welcome to the IIgs"
      Super Hi-Res startup screen.** The IIgs boots a SmartPort disk end-to-end.
- [x] Regression tests: `parses_2mg_header_offsets`, `smartport_boot_loads_block0`;
      updated `smartport_2mg_format` and `smartport_firmware_stub_installed`.

## Fixed — banks $80-$DF aliased $00-$5F  ✅
Removed the `$80-$DF` → `$00-$5F` mirror (real bug: made GS/OS mis-size RAM and
corrupt bank $00). Verified vs GSplus dummy-memory. Did NOT fully fix the GS/OS
crash below, but is a correctness fix.

## NEXT — GS/OS crashes after the "Welcome" screen (deep runaway)  ⛔
GS/OS reaches the "Welcome to the IIgs" SHR screen, then the CPU runs away into
GS/OS's memory-fill catcher pattern (`AF 57 00 84` = `LDA $840057`) in bank-0
low memory, with the stack pointer corrupted into the `$C0xx` I/O region. By the
time any symptom is detectable (SP in `$C0xx`, executing `$AF`/`$84` fill, BRK
dispatch through the garbage `$03F0` vector) the CPU has already been lost for
600+ instructions — the true divergence is far upstream. Iterative single-symptom
tracing isn't converging; this needs a **reference-trace diff** against GSplus
(run the same ROM+disk in GSplus with instruction logging, diff PC streams to
find the first divergence). Likely suspects: the unimplemented clock chip
(`$C033`/`$C034` RTC + BRAM — GS/OS reads config from it), a GS/OS toolset call
hitting unimplemented hardware, or a subtle 65C816/memory edge case.

## (was) P16 → GS/OS handoff note — superseded
GS/OS reaches the "Welcome to the IIgs" Super Hi-Res screen (SHR on, no BRK),
then the P16 loader crashes: code at `$00/2568` does `JMP $0080`, but bank-0
`$0080+` is filled with a repeating `AF 57 00 84` (`LDA $840057`) pattern — GS/OS's
wild-jump catch-fill — so the CPU runs garbage until a `BRK` at `$00C3`, which the
`$C074`→`$E10010`→`$FFB7CC` interrupt handler catches and loops. Not a Monitor
drop. Root cause is upstream of the jump: GS/OS expected valid code at `$0080`
that was never installed (likely a SmartPort/ProDOS read subtlety, an ALTZP/zero-
page-shadow mismatch, or a missing GS/OS prerequisite — clock chip RTC/BRAM is
still unimplemented). Trace back from the `JMP $0080` at `$00/2568` to find what
should have populated `$0080`.

## LATER — Ensoniq DOC sound (games are silent)  ⛔
The `synth_ensoniq_audio` path exists (`fill_audio`) but games produce no sound.
Likely: the DOC sound-RAM isn't populated by the firmware writes (verify
`$C03C-$C03F` → `ensoniq.write_data` routes to sound RAM in RAM-access mode), and
the speaker/DOC/Mockingboard streams append separate blocks to the ring buffer
rather than mixing per-sample (fine when one source is active, wrong when both
are). The IIgs has no boot chime, so silence on the "Welcome" screen is expected;
test with a game (e.g. Silpheed) that drives the DOC.

## (resolved) old boot-scan note
Both ROMs reach the banner then drop to the Monitor via `BRK $00/0003` — the
firmware isn't scanning/booting slot 5 (SmartPort): slot-5 firmware ($C500) is
never executed and the SmartPort WDM trap never fires. Likely needs: correct
boot-slot handling (BRAM startup slot / the real BRAM layout is a guess), the
slot-firmware scan finding the SmartPort signature, and the SmartPort block-0
boot handoff. Secondary: clock-chip ($C033/$C034) BRAM + RTC (currently the
ADB-BRAM shortcut was removed; BRAM reads go through the unimplemented clock).

## (old) IWM self-test note — resolved above
ROM 03 now runs init and reaches a polling loop at `$FF/4720` (DBR=`$E1`) that
reads the IWM registers `$C0E8-$C0EF` (slot 6) — motor/phase/Q6/Q7 handshake:
```
4717: LDA $C0EE / AND #$20 / BNE      ; wait IWM status bit 5 clear
4720: STY $C0EF / TYA / EOR $C0EE / AND #$1F / BNE 4720
```
Those registers are unimplemented (return 0) so the loop never exits. Implementing
the IWM (even a minimal "no 5.25 drive" model that satisfies the self-test) is the
next step to reach the SmartPort/GS-OS boot path.

## Deferred (secondary — separate task)
- Ensoniq DOC IRQ wiring; faithful LC bank1/bank2 model; IWM (5.25/3.5 boot);
  SCC. Full GS/OS desktop boot needs SmartPort/IWM block-boot + these.
- Exact per-scan-line interrupt timing (current model raises the scan-line
  interrupt once per frame if any SCB requests it, rather than at the precise
  scanline — sufficient for heartbeat-driven software).

## Review

Both blockers fixed and verified against real ROMs.

- **memory.rs** — `normalize_rom_layout` swaps the two 64KB halves of a 128KB
  image when the bank-`$FF` reset vector is blank but the other half is valid.
  Version detection moved after normalization; 256KB (ROM 03) images untouched.
- **bus.rs** — extracted `io_read_reg`/`io_write_reg` (shared by slow, fast, and
  ROM banks) and added `read_fast_bank`/`write_fast_bank` (+ `*_language_card_fast`)
  so banks `$E0`/`$E1` decode the I/O aperture, slot ROM, and language card
  instead of reading raw fast RAM.

Before: ROM 01 dead-looped at `$00/0000`; a correct ROM hung at `$FF/B53A`
(1–2 distinct PCs). After: reset `$FA62` → native mode → self-test running,
24,577 distinct PCs sampled, text written to `$E0` screen memory.

Checks: fmt clean, clippy `-D warnings` clean, release build OK, full test
suite passes (3 new IIgs regression tests among them).

**Not yet booting to the desktop** — that needs Blocker 3 (heartbeat ¼/1-sec +
VGC scanline interrupts) and the secondary items, tracked as a follow-up task.
Recommend a manual GUI launch with `machine_type = AppleIIgs` to see the
self-test on screen.
