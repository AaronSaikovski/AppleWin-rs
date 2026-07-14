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

## NEXT BLOCKER — IWM (5.25" disk controller) self-test  ⛔
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
