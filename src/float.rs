//! IBM hexadecimal floating point for the AP-101S (IBM 85-C67-001 §8).
//!
//! Format (§8.1-8.2): sign bit, 7-bit characteristic in excess-64 notation,
//! and a sign-magnitude hexadecimal fraction with the radix point left of
//! the high-order digit. Short = fullword with a 6-digit (24-bit) fraction;
//! long = register/storage pair with a 14-digit (56-bit) fraction.
//!
//! Semantics implemented from §8.9-8.29:
//! - Addition/subtraction prealign the smaller-characteristic fraction
//!   right (guard digits retained and participating, §8.10), then
//!   postnormalize. A high-order carry shifts right one digit.
//! - A zero-fraction input is treated as a true zero regardless of sign or
//!   characteristic (§8.3); arithmetic writes true zeros for zero results,
//!   whose sign is always positive (§8.9).
//! - Exceptions (§8.8): exponent overflow (result characteristic > 127,
//!   operands unchanged), exponent underflow (characteristic < 0 — result
//!   is a true zero when the underflow mask is off, no result written when
//!   it's on), significance (zero result fraction in add/subtract — a true
//!   zero is written regardless of the mask), and floating-point divide
//!   (division suppressed).
//!
//! Everything works internally on [`Unpacked`] values with 56-bit
//! fractions; short operands occupy the top 24 bits so one code path
//! serves both precisions.

/// Fraction bits used internally (14 hex digits).
const FRAC_BITS: u32 = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    Short,
    Long,
}

impl Precision {
    /// Fraction digits carried by this precision.
    fn digits(self) -> u32 {
        match self {
            Precision::Short => 6,
            Precision::Long => 14,
        }
    }

    /// Mask keeping this precision's digits of a 56-bit fraction.
    fn mask(self) -> u64 {
        !0u64 << (FRAC_BITS - 4 * self.digits()) & ((1 << FRAC_BITS) - 1)
    }
}

/// Outcome of an arithmetic operation (§8.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpEvent {
    None,
    /// Exponent overflow: interrupt, no result written.
    Overflow,
    /// Exponent underflow: result is true zero if the mask is off;
    /// interrupt with no result written if the mask is on.
    Underflow,
    /// Significance (add/subtract zero result): true zero is written
    /// regardless; interrupt if the mask is on.
    Significance,
    /// Divide by zero-fraction divisor: suppressed, nothing written.
    DivideException,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unpacked {
    pub neg: bool,
    /// Characteristic 0..=127 while packed; may leave that range in
    /// intermediate results before the overflow/underflow checks.
    pub ch: i32,
    /// Fraction, top-aligned: bit 55 down; value = frac * 16^-14.
    pub frac: u64,
}

pub const TRUE_ZERO: Unpacked = Unpacked { neg: false, ch: 0, frac: 0 };

impl Unpacked {
    pub fn is_zero(&self) -> bool {
        self.frac == 0
    }
}

/// Unpack a short operand. A zero fraction is a true zero regardless of
/// sign/characteristic (§8.3).
pub fn unpack_short(w: u32) -> Unpacked {
    let frac = ((w & 0x00FF_FFFF) as u64) << 32;
    if frac == 0 {
        TRUE_ZERO
    } else {
        Unpacked { neg: w & 0x8000_0000 != 0, ch: ((w >> 24) & 0x7F) as i32, frac }
    }
}

/// Unpack a long operand from its two fullwords (high = sign/char/6 digits,
/// low = 8 more digits; §8.1, Figure 8-1).
pub fn unpack_long(hi: u32, lo: u32) -> Unpacked {
    let frac = (((hi & 0x00FF_FFFF) as u64) << 32) | lo as u64;
    if frac == 0 {
        TRUE_ZERO
    } else {
        Unpacked { neg: hi & 0x8000_0000 != 0, ch: ((hi >> 24) & 0x7F) as i32, frac }
    }
}

/// Pack to a short fullword, truncating the fraction (no rounding, §8).
pub fn pack_short(u: Unpacked) -> u32 {
    if u.frac & Precision::Short.mask() == 0 {
        return 0; // true zero
    }
    ((u.neg as u32) << 31) | (((u.ch as u32) & 0x7F) << 24) | ((u.frac >> 32) as u32)
}

pub fn pack_long(u: Unpacked) -> (u32, u32) {
    if u.frac == 0 {
        return (0, 0);
    }
    (
        ((u.neg as u32) << 31) | (((u.ch as u32) & 0x7F) << 24) | ((u.frac >> 32) as u32 & 0x00FF_FFFF),
        u.frac as u32,
    )
}

/// Postnormalize: shift the fraction left one digit at a time, reducing the
/// characteristic, until the high-order digit (within `p`'s precision) is
/// nonzero (§8.3). Returns Underflow if the characteristic drops below 0.
fn normalize(mut u: Unpacked, p: Precision) -> (Unpacked, FpEvent) {
    if u.frac & p.mask() == 0 {
        return (TRUE_ZERO, FpEvent::None);
    }
    while u.frac & (0xF << (FRAC_BITS - 4)) == 0 {
        u.frac <<= 4;
        u.ch -= 1;
    }
    if u.ch < 0 {
        (u, FpEvent::Underflow)
    } else {
        (u, FpEvent::None)
    }
}

/// Add (or subtract, via `negate_b`) with one guard digit participating
/// (§8.9/8.10). Returns the normalized result and any exception event.
pub fn add(a: Unpacked, b: Unpacked, negate_b: bool, p: Precision) -> (Unpacked, FpEvent) {
    let mut b = b;
    if negate_b && !b.is_zero() {
        b.neg = !b.neg;
    }
    // Work in guard-extended magnitudes: fraction << 4 would overflow 60
    // bits of u64 comfortably; use i128 signed magnitudes with one guard
    // digit below the 56-bit fraction.
    let (hi, lo) = if a.ch >= b.ch { (a, b) } else { (b, a) };
    let shift = (hi.ch - lo.ch) as u32;
    // Guard-extended: 4 extra low bits. Digits shifted past the guard are
    // lost (truncated), as with the hardware's finite guard (§8.10).
    let hi_m = (hi.frac as i128) << 4;
    let lo_m = if shift >= 15 { 0 } else { ((lo.frac as i128) << 4) >> (4 * shift) };
    let signed =
        (if hi.neg { -hi_m } else { hi_m }) + (if lo.neg { -lo_m } else { lo_m });
    let neg = signed < 0;
    let mut mag = signed.unsigned_abs() as u128;
    let mut ch = hi.ch;
    // High-order carry: shift right one digit, characteristic +1 (§8.9).
    if mag >> (FRAC_BITS + 4) != 0 {
        mag >>= 4;
        ch += 1;
        if ch > 127 {
            return (TRUE_ZERO, FpEvent::Overflow);
        }
    }
    if mag == 0 {
        // Significance: true zero written regardless of the mask (§8.8).
        return (TRUE_ZERO, FpEvent::Significance);
    }
    // Drop the guard digit only after normalization so a left shift can
    // recover it (guard digits "increase the precision of the final
    // result", §8.1).
    let mut u = Unpacked { neg, ch, frac: 0 };
    let mut guarded = mag; // 60-bit quantity
    if guarded & (0xFu128 << FRAC_BITS) == 0 {
        // normalize within the guard-extended value
        while guarded != 0 && guarded & (0xFu128 << FRAC_BITS) == 0 {
            guarded <<= 4;
            u.ch -= 1;
        }
    }
    u.frac = (guarded >> 4) as u64;
    // Truncate to precision and re-check zero.
    u.frac &= p.mask();
    if u.frac == 0 {
        return (TRUE_ZERO, FpEvent::Significance);
    }
    if u.ch < 0 {
        return (u, FpEvent::Underflow);
    }
    if u.ch > 127 {
        return (TRUE_ZERO, FpEvent::Overflow);
    }
    (u, FpEvent::None)
}

/// Algebraic compare per the rules of normalized floating-point
/// subtraction (§8.11/8.12): -1, 0, +1. Zero fractions compare equal
/// regardless of sign or characteristic.
pub fn compare(a: Unpacked, b: Unpacked, p: Precision) -> i32 {
    let (d, _) = add(a, b, true, p);
    if d.is_zero() {
        0
    } else if d.neg {
        -1
    } else {
        1
    }
}

/// Multiply (§8.24/8.25): characteristics add less 64; exact fraction
/// product, postnormalized, truncated to precision.
pub fn multiply(a: Unpacked, b: Unpacked, p: Precision) -> (Unpacked, FpEvent) {
    if a.is_zero() || b.is_zero() {
        // True zero forced; no overflow/underflow can occur (§8.24).
        return (TRUE_ZERO, FpEvent::None);
    }
    let prod = (a.frac as u128) * (b.frac as u128); // 112 bits, * 16^-28
    let mut ch = a.ch + b.ch - 64;
    let mut frac_ext = prod; // top digit at bit 108..111
    // Align the product's leading digit to the top of the 112-bit window
    // (normalized inputs need at most one shift; unnormalized inputs are
    // accepted and simply postnormalize further, §8.3).
    while frac_ext != 0 && frac_ext >> 108 == 0 {
        frac_ext <<= 4;
        ch -= 1;
    }
    let mut u = Unpacked { neg: a.neg != b.neg, ch, frac: (frac_ext >> 56) as u64 };
    u.frac &= p.mask();
    if u.frac == 0 {
        // All digits of the intermediate product zero: true zero, no
        // interruption (§8.24).
        return (TRUE_ZERO, FpEvent::None);
    }
    if u.ch > 127 {
        return (TRUE_ZERO, FpEvent::Overflow);
    }
    if u.ch < 0 {
        return (u, FpEvent::Underflow);
    }
    (u, FpEvent::None)
}

/// Divide (§8.15/8.16): characteristic difference plus 64; quotient
/// fraction truncated to precision; no remainder.
pub fn divide(a: Unpacked, b: Unpacked, p: Precision) -> (Unpacked, FpEvent) {
    if b.is_zero() {
        return (TRUE_ZERO, FpEvent::DivideException);
    }
    if a.is_zero() {
        // True zero quotient without underflow/overflow interrupts (§8.15).
        return (TRUE_ZERO, FpEvent::None);
    }
    let mut ch = a.ch - b.ch + 64;
    // q = fa/fb in (0, 16) for nonzero fractions; produce 56 fraction bits.
    let mut q = ((a.frac as u128) << FRAC_BITS) / b.frac as u128;
    // q holds fa/fb * 2^56; if >= 1.0 (i.e. 2^56), shift right one digit.
    if q >> FRAC_BITS != 0 {
        q >>= 4;
        ch += 1;
    }
    let mut u = Unpacked { neg: a.neg != b.neg, ch, frac: q as u64 };
    // Normalize (unnormalized inputs can yield leading zero digits).
    let (n, ev) = normalize(u, p);
    u = n;
    if ev == FpEvent::Underflow {
        return (u, FpEvent::Underflow);
    }
    u.frac &= p.mask();
    if u.frac == 0 {
        return (TRUE_ZERO, FpEvent::None);
    }
    if u.ch > 127 {
        return (TRUE_ZERO, FpEvent::Overflow);
    }
    if u.ch < 0 {
        return (u, FpEvent::Underflow);
    }
    (u, FpEvent::None)
}

/// Normalize a value for LOAD-type outputs that DO normalize (MVS output,
/// §8.23) — returns Underflow when normalization drops the characteristic
/// below zero.
pub fn normalize_value(u: Unpacked, p: Precision) -> (Unpacked, FpEvent) {
    normalize(u, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Short-format spot values: 1.0 = 0x41100000 (§8.21 LFLI table),
    // 2.0 = 0x41200000, 15.0 = 0x41F00000, 0.5 = 0x40800000,
    // 16.0 = 0x42100000.

    #[test]
    fn pack_unpack_round_trip() {
        for w in [0x4110_0000u32, 0xC110_0000, 0x4080_0000, 0x7FFF_FFFF] {
            assert_eq!(pack_short(unpack_short(w)), w);
        }
        // zero fraction unpacks to true zero whatever the char/sign
        assert_eq!(unpack_short(0xC500_0000), TRUE_ZERO);
    }

    #[test]
    fn add_basic() {
        let one = unpack_short(0x4110_0000);
        let two = unpack_short(0x4120_0000);
        let (r, ev) = add(one, two, false, Precision::Short);
        assert_eq!(ev, FpEvent::None);
        assert_eq!(pack_short(r), 0x4130_0000); // 3.0
        // 1 + 1 = 2
        let (r, _) = add(one, one, false, Precision::Short);
        assert_eq!(pack_short(r), 0x4120_0000);
        // 1 - 1 = true zero with significance event
        let (r, ev) = add(one, one, true, Precision::Short);
        assert_eq!(r, TRUE_ZERO);
        assert_eq!(ev, FpEvent::Significance);
    }

    #[test]
    fn add_prealignment_and_carry() {
        // 15.0 + 1.0 = 16.0: fraction carry forces right shift, char +1.
        let fifteen = unpack_short(0x41F0_0000);
        let one = unpack_short(0x4110_0000);
        let (r, ev) = add(fifteen, one, false, Precision::Short);
        assert_eq!(ev, FpEvent::None);
        assert_eq!(pack_short(r), 0x4210_0000); // 16.0
        // 16.0 - 1.0 = 15.0 exercises the guard digit: 0x42100000 aligned
        // against 0x41100000 shifted right one digit.
        let sixteen = unpack_short(0x4210_0000);
        let (r, _) = add(sixteen, one, true, Precision::Short);
        assert_eq!(pack_short(r), 0x41F0_0000);
    }

    #[test]
    fn multiply_divide_basic() {
        let two = unpack_short(0x4120_0000);
        let half = unpack_short(0x4080_0000);
        let (r, ev) = multiply(two, half, Precision::Short);
        assert_eq!(ev, FpEvent::None);
        assert_eq!(pack_short(r), 0x4110_0000); // 1.0
        let (r, ev) = divide(two, two, Precision::Short);
        assert_eq!(ev, FpEvent::None);
        assert_eq!(pack_short(r), 0x4110_0000);
        let one = unpack_short(0x4110_0000);
        let (r, _) = divide(one, two, Precision::Short);
        assert_eq!(pack_short(r), 0x4080_0000); // 0.5
        // divide by zero fraction
        let (_, ev) = divide(one, TRUE_ZERO, Precision::Short);
        assert_eq!(ev, FpEvent::DivideException);
    }

    #[test]
    fn exponent_overflow_and_underflow() {
        // max char multiply: char 127 * char 127 -> 127+127-64 = 190
        let big = unpack_short(0x7F10_0000);
        let (_, ev) = multiply(big, big, Precision::Short);
        assert_eq!(ev, FpEvent::Overflow);
        let tiny = unpack_short(0x0010_0000); // char 0
        let (_, ev) = multiply(tiny, tiny, Precision::Short);
        assert_eq!(ev, FpEvent::Underflow);
    }

    #[test]
    fn compare_rules() {
        let one = unpack_short(0x4110_0000);
        let two = unpack_short(0x4120_0000);
        let neg_one = unpack_short(0xC110_0000);
        assert_eq!(compare(one, two, Precision::Short), -1);
        assert_eq!(compare(two, one, Precision::Short), 1);
        assert_eq!(compare(one, one, Precision::Short), 0);
        assert_eq!(compare(neg_one, one, Precision::Short), -1);
        // zero fractions compare equal even with different sign/char (§8.12)
        assert_eq!(
            compare(unpack_short(0xC500_0000), unpack_short(0x1200_0000), Precision::Short),
            0
        );
    }

    #[test]
    fn long_precision() {
        let a = unpack_long(0x4110_0000, 0x0000_0001);
        let b = unpack_long(0x4110_0000, 0x0000_0000);
        let (r, _) = add(a, b, true, Precision::Long);
        // difference is the last bit: 16^-14 * 16^1 => normalized
        assert!(!r.is_zero());
        assert_eq!(compare(a, b, Precision::Long), 1);
        // in SHORT precision those low bits are invisible
        assert_eq!(
            compare(
                unpack_short(0x4110_0000),
                unpack_short(0x4110_0000),
                Precision::Short
            ),
            0
        );
    }
}
