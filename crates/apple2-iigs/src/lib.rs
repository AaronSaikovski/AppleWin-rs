//! Apple IIgs emulation.
//!
//! Provides the 65C816 CPU, IIgs memory bus with Mega II compatibility,
//! Super Hi-Res video, Ensoniq DOC 5503 audio, ADB input, and SmartPort
//! disk I/O — everything needed to emulate an Apple IIgs.

pub mod adb;
pub mod bram;
pub mod bus;
pub mod cpu65816;
pub mod emulator;
pub mod ensoniq;
pub mod fpi;
pub mod mega2;
pub mod memory;
pub mod shadowing;
pub mod shr;
pub mod smartport;
