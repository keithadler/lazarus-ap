//! FCM (flight computer memory) image loading — phase 6.
//!
//! The Virtual AGC Shuttle toolchain (HALSFC compiler -> lnk101 linker)
//! produces `.fcm` files: raw big-endian 16-bit halfwords, loaded into
//! main storage starting at halfword address 0, with the entry point
//! carried in a companion symbols JSON (`entryPoint` field); the
//! emulator then sets the PSW's instruction address there and clears
//! the wait state. Conventions verified against yaGPC2's loader
//! (`ageharness.c` load_fcm/load_symbols, `mcm.c` mcm_load16).
//!
//! Only `entryPoint` is consumed from the JSON here; the full symbol
//! schema (and the HAL/S runtime's host-side WRITE interception that
//! yaGPC2's halucp layer provides) is later work — see ROADMAP.md.

use crate::cpu::Cpu;
use crate::mem::{AddressError, Memory};

/// Load an FCM image: big-endian halfwords at address 0. Odd trailing
/// bytes are ignored (the files are halfword streams).
pub fn load_fcm(mem: &mut Memory, bytes: &[u8]) -> Result<u32, AddressError> {
    let mut n = 0;
    for (i, hw) in bytes.chunks_exact(2).enumerate() {
        mem.write_h(i as u32, u16::from_be_bytes([hw[0], hw[1]]))?;
        n += 1;
    }
    Ok(n)
}

/// Extract the numeric `entryPoint` from an lnk101 symbols JSON. A
/// deliberately minimal scanner: the full schema is unverified, and
/// only the entry point is needed to run.
pub fn entry_point(symbols_json: &str) -> Option<u32> {
    let i = symbols_json.find("\"entryPoint\"")?;
    let rest = &symbols_json[i + "\"entryPoint\"".len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    if let Some(hex) = rest.strip_prefix("\"0x").or_else(|| rest.strip_prefix("\"0X")) {
        let end = hex.find('"')?;
        return u32::from_str_radix(&hex[..end], 16).ok();
    }
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Boot a CPU from an FCM image and optional symbols JSON, mirroring
/// yaGPC2: image at 0, IC at the entry point (or 0), wait cleared.
pub fn boot(cpu: &mut Cpu, fcm: &[u8], symbols_json: Option<&str>) -> Result<(), AddressError> {
    load_fcm(&mut cpu.mem, fcm)?;
    let entry = symbols_json.and_then(entry_point).unwrap_or(0);
    cpu.psw.ic = entry as u16;
    cpu.psw.wait = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::assemble;
    use crate::{Halt, Memory};

    #[test]
    fn entry_point_parsing() {
        assert_eq!(entry_point(r#"{"entryPoint": 292, "x": 1}"#), Some(292));
        assert_eq!(entry_point(r#"{"entryPoint":"0x124"}"#), Some(0x124));
        assert_eq!(entry_point(r#"{"symbols": []}"#), None);
    }

    #[test]
    fn fcm_round_trip_runs() {
        // Assemble a small program, serialize it the way lnk101 lays out
        // an FCM (big-endian halfwords from 0), reload through the FCM
        // path, and run it.
        let src = "
        ORG  0x124
        LFXI 1,6
        AHI  1,36
        STH  1,RESULT
DONE:   B    DONE
RESULT: DC   H(0)
";
        let prog = assemble(src).unwrap();
        let mut image = Memory::new(0x400);
        prog.load(&mut image).unwrap();
        let result_addr = prog.label("RESULT").unwrap();
        let mut bytes = Vec::new();
        for a in 0..0x200u32 {
            bytes.extend_from_slice(&image.read_h(a).unwrap().to_be_bytes());
        }

        let mut cpu = crate::Cpu::new(Memory::new(0x400));
        boot(&mut cpu, &bytes, Some(r#"{"entryPoint": "0x124"}"#)).unwrap();
        assert_eq!(cpu.psw.ic, 0x124);
        assert!(matches!(cpu.run(100), Halt::SelfLoop { .. }));
        assert_eq!(cpu.mem.read_h(result_addr).unwrap(), 42);
    }
}
