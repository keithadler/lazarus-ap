//! AP-101S main storage.
//!
//! The architecture transmits information in 16-bit halfwords; a fullword is
//! two consecutive halfwords, addressed by its leftmost (lowest-address)
//! halfword. Halfword locations are consecutively numbered from 0 using a
//! 19-bit binary address (max 2^19 halfword addresses).
//! [IBM 85-C67-001 §2.1.1 "Information Formats", §2.1.2 "Addressing"]
//!
//! The AP-101S "makes provision to address 262,144 fullwords, and the AP-101S
//! space shuttle hardware implementation provides full addressing capability"
//! [ibid. §2.5.1.1 "Instruction Address"], hence the default size of
//! 2^19 halfwords (= 256K fullwords = 1 MiB). Smaller configurations are
//! supported via [`Memory::new`].
//!
//! Unlike earlier AP-101 models, the AP-101S does **not** require fullword
//! instructions or fullword/doubleword operands to sit on even halfword
//! boundaries [ibid. §2.1.3 "Information Positioning"], so no alignment
//! checks are performed here.
//!
//! Bit numbering follows the manual: bits are numbered 0..15 (halfword) or
//! 0..31 (fullword) from the most significant end [ibid. Figure 2-1], i.e.
//! storage is big-endian. `read_byte`/`write_byte` expose a big-endian byte
//! view (byte 0 = bits 0-7 of halfword 0) as an emulator convenience for
//! loading images; the AP-101S itself has no byte addressing — the halfword
//! is the smallest addressed unit.

/// Default storage size in halfwords: the full 19-bit address space.
pub const DEFAULT_SIZE_HALFWORDS: usize = 1 << 19;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressError {
    /// Offending 19-bit (or larger) halfword address.
    pub addr: u32,
    /// Installed size in halfwords.
    pub size: u32,
}

pub struct Memory {
    hw: Vec<u16>,
}

impl Memory {
    /// A memory of `halfwords` 16-bit locations, all zero.
    pub fn new(halfwords: usize) -> Memory {
        assert!(
            halfwords <= DEFAULT_SIZE_HALFWORDS,
            "AP-101S addressing is limited to 2^19 halfwords"
        );
        Memory { hw: vec![0; halfwords] }
    }

    /// Full-size (2^19 halfword) memory.
    pub fn full() -> Memory {
        Memory::new(DEFAULT_SIZE_HALFWORDS)
    }

    pub fn size_halfwords(&self) -> u32 {
        self.hw.len() as u32
    }

    fn check(&self, addr: u32) -> Result<usize, AddressError> {
        let i = addr as usize;
        if i < self.hw.len() {
            Ok(i)
        } else {
            Err(AddressError { addr, size: self.size_halfwords() })
        }
    }

    pub fn read_h(&self, addr: u32) -> Result<u16, AddressError> {
        Ok(self.hw[self.check(addr)?])
    }

    pub fn write_h(&mut self, addr: u32, v: u16) -> Result<(), AddressError> {
        let i = self.check(addr)?;
        self.hw[i] = v;
        Ok(())
    }

    /// Fullword at `addr`: the halfword at `addr` is bits 0-15 (most
    /// significant), the halfword at `addr+1` is bits 16-31.
    pub fn read_f(&self, addr: u32) -> Result<u32, AddressError> {
        let hi = self.read_h(addr)? as u32;
        let lo = self.read_h(addr.wrapping_add(1))? as u32;
        Ok((hi << 16) | lo)
    }

    pub fn write_f(&mut self, addr: u32, v: u32) -> Result<(), AddressError> {
        self.write_h(addr, (v >> 16) as u16)?;
        self.write_h(addr.wrapping_add(1), v as u16)
    }

    /// Doubleword: fullword at `addr` is bits 0-31, fullword at `addr+2`
    /// is bits 32-63.
    pub fn read_d(&self, addr: u32) -> Result<u64, AddressError> {
        let hi = self.read_f(addr)? as u64;
        let lo = self.read_f(addr.wrapping_add(2))? as u64;
        Ok((hi << 32) | lo)
    }

    pub fn write_d(&mut self, addr: u32, v: u64) -> Result<(), AddressError> {
        self.write_f(addr, (v >> 32) as u32)?;
        self.write_f(addr.wrapping_add(2), v as u32)
    }

    /// Big-endian byte view (emulator convenience; not an ISA feature).
    /// Byte address `2h` is bits 0-7 of halfword `h`; `2h+1` is bits 8-15.
    pub fn read_byte(&self, byte_addr: u32) -> Result<u8, AddressError> {
        let h = self.read_h(byte_addr / 2)?;
        Ok(if byte_addr % 2 == 0 { (h >> 8) as u8 } else { h as u8 })
    }

    pub fn write_byte(&mut self, byte_addr: u32, v: u8) -> Result<(), AddressError> {
        let i = self.check(byte_addr / 2)?;
        let h = self.hw[i];
        self.hw[i] = if byte_addr % 2 == 0 {
            (h & 0x00FF) | ((v as u16) << 8)
        } else {
            (h & 0xFF00) | v as u16
        };
        Ok(())
    }

    /// Load a slice of halfwords starting at `addr` (program loader).
    pub fn load_halfwords(&mut self, addr: u32, words: &[u16]) -> Result<(), AddressError> {
        for (i, w) in words.iter().enumerate() {
            self.write_h(addr + i as u32, *w)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_endian_layout() {
        let mut m = Memory::new(16);
        m.write_f(4, 0x1234_5678).unwrap();
        // Fullword's most significant halfword lives at the lower address.
        assert_eq!(m.read_h(4).unwrap(), 0x1234);
        assert_eq!(m.read_h(5).unwrap(), 0x5678);
        // Byte view: byte 0 of a halfword is its most significant 8 bits.
        assert_eq!(m.read_byte(8).unwrap(), 0x12);
        assert_eq!(m.read_byte(9).unwrap(), 0x34);
        assert_eq!(m.read_byte(10).unwrap(), 0x56);
        assert_eq!(m.read_byte(11).unwrap(), 0x78);
    }

    #[test]
    fn doubleword_layout() {
        let mut m = Memory::new(16);
        m.write_d(0, 0x0102_0304_0506_0708).unwrap();
        assert_eq!(m.read_f(0).unwrap(), 0x0102_0304);
        assert_eq!(m.read_f(2).unwrap(), 0x0506_0708);
        assert_eq!(m.read_d(0).unwrap(), 0x0102_0304_0506_0708);
    }

    #[test]
    fn odd_fullword_addresses_allowed() {
        // §2.1.3: the AP-101S allows fullword operands on odd halfword
        // boundaries.
        let mut m = Memory::new(16);
        m.write_f(3, 0xAABB_CCDD).unwrap();
        assert_eq!(m.read_f(3).unwrap(), 0xAABB_CCDD);
    }

    #[test]
    fn out_of_range_is_error() {
        let mut m = Memory::new(8);
        assert!(m.read_h(8).is_err());
        assert!(m.write_f(7, 0).is_err()); // second halfword out of range
    }
}
