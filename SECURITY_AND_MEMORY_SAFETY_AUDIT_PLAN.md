# Security and Memory Safety Audit

## Context
Perform a security-focused code review of the Rust codebase, specifically targeting the low-level emulation engine. The goal is to identify potential memory safety issues, `unsafe` misuse, resource exhaustion vectors, or logic vulnerabilities that could lead to unexpected behavior or crashes.

## Approach

### 1. Audit `unsafe` blocks
- **ROM Mapping**: Verify that `include_bytes!` usage in `mb_firmware.rs` and `mouse.rs` is safe (ensure the slice-to-array cast matches the actual file size).
- **Framebuffer Slicing**: Review `framebuffer.rs` for `std::slice::from_raw_parts` usage, ensuring alignment and size invariants are maintained.

### 2. Analyze Memory Bus and Addressing
- **Page Table Safety**: Verify the `Bus::rebuild_page_tables` logic for correct address translation and bounds.
- **Memory Mapping**: Check that `main_ram` and `aux_ram` access via `PageSrc`/`PageDst` cannot result in out-of-bounds indexing.
- **Auxiliary Memory (RamWorks III)**: Audit `rw3_switch` and `aux_read/write` for potential resource exhaustion (large allocations) or out-of-bounds access if `rw3_active` is manipulated.

### 3. Evaluate Peripheral and I/O Logic
- **Soft Switch Side-Effects**: Review `soft_switches.rs` for any state transitions that could lead to invalid memory configurations or infinite loops.
- **Card/Slot Management**: Audit `CardManager` and the `Card` trait implementation for type-safety and potential injection points.

### 4. File and Data Handling
- **Disk Image Loading**: Review `disk_util.rs` and `Bus::load_disk` for vulnerabilities in decompression or header parsing (e.g. path traversal or buffer overflows).

## Critical files & anchors
- `crates/apple2-core/src/bus/mod.rs`: `Bus` implementation (read/write, page tables, rw3 logic).
- `crates/apple2-video/src/framebuffer.rs`: `pixels_as_bytes` unsafe slice creation.
- `crates/apple2-core/src/card.rs`: `CardManager` and slot access logic.
- `crates/apple2-core/src/bus/soft_switches.rs`: Soft switch handlers and memory mode transitions.

## Verification
- **Manual Code Audit**: Verification of the findings listed in the approach.
- **Sanity Check**: Ensure all `unsafe` blocks have valid `// SAFETY` comments and satisfy their invariants.
