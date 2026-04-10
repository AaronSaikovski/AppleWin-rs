//! Apple II debugger — pure logic, no Win32 / UI dependencies.
//!
//! Phase 5 implementation. References:
//!   source/Debugger/Debug.cpp
//!   source/Debugger/Debugger_Disassembler.cpp
//!   source/Debugger/Debugger_Assembler.cpp
//!   source/Debugger/Debugger_Symbols.cpp
//!   source/Debugger/Debugger_Commands.cpp

pub mod breakpoint;
pub mod commands;
pub mod disasm;
pub mod display;
pub mod softswitch;
pub mod state;
pub mod symbols;
pub mod trace;
pub mod watches;

pub use state::DebuggerState;
