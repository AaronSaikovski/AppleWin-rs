# Lessons

## 2026-07-07 — Reproduce before fixing (Airheart startup screech)

**What happened:** Asked to fix a "screeching sound on startup" on the //c, I
found a real emulation bug by code review (IIe VBL semantics used on the //c),
fixed it with tests — and it wasn't the cause. The actual bug was in the GUI
speaker synthesis: no sub-sample averaging, so PWM audio (toggles faster than
one per output sample) aliased into a screech.

**Rules for next time:**
- For a "wrong output" bug (sound, video, timing), reproduce and capture the
  actual data stream FIRST, before committing to a hypothesis. Here: a 40-line
  headless probe booting the user's own disk image captured the speaker-toggle
  stream and pinpointed the layer (game generated a clean PWM stream → bug had
  to be in host rendering, not emulation).
- Ask early which title/program exhibits the bug ("some games" → get one name).
  The game name (Airheart) instantly narrowed the technique involved.
- A plausible bug found by review is not necessarily THE bug. Fix it if it's
  real, but keep the report open until the symptom is verified fixed
  end-to-end.
- Instrumentation to keep: scratchpad probe pattern (boot image headless, drain
  `bus.speaker_toggles` per frame, interval stats per second) and offline
  render comparison (simulate the app's synthesis over a captured stream,
  write WAVs, compare RMS / zero-crossing rate).
