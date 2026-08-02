//! Lazarus AP — an emulator of the IBM AP-101S, the Space Shuttle's
//! general-purpose computer (GPC).
//!
//! Phase 1: CPU + main storage, executing the "Shuttle instruction set" as
//! documented in IBM's *Space Shuttle Model AP-101S Principles of Operation
//! with Shuttle Instruction Set* (IBM 85-C67-001). Every implemented
//! instruction's encoding and semantics trace to that document; see
//! docs/ISA_STATUS.md for per-instruction verification status and
//! docs/SOURCES.md for the sources themselves.
//!
//! What this phase deliberately does not implement: floating point execution,
//! I/O (IOP/BCE/MSC), interrupts, storage protection, the DSE-loading and
//! stack/supervisor instructions. Executing one of those opcodes returns a
//! [`Trap`](cpu::Trap) rather than silently guessing.

pub mod asm;
pub mod cpu;
pub mod decode;
pub mod demo;
pub mod deu;
pub mod fcm;
pub mod float;
pub mod gpc;
pub mod halucp;
pub mod iop;
pub mod mem;
pub mod psw;
pub mod trace;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use cpu::{Cpu, Halt, IoSubsystem, PcResponse, Trap};
pub use mem::Memory;
pub use psw::Psw;
