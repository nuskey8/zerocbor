use core::hint::cold_path;

use alloc::vec::Vec;

use crate::Error;
use crate::Result;
use crate::consts::*;

/// The largest number of bytes any single CBOR head or scalar occupies: a
/// 1-byte head plus an 8-byte payload. Bulk slice writers reserve this once for
/// a whole slice rather than checking bounds per element.
const MAX_ENCODED_INT: usize = 9;

/// Encodes a CBOR argument with its major type directly into the destination.
///
/// # Safety
/// `output` must have room for the encoded head (at most nine bytes).
#[inline(always)]
unsafe fn encode_argument_at(output: *mut u8, major_type: u8, value: u64) -> usize {
    unsafe {
        if value <= 23 {
            output.write(major_type | value as u8);
            1
        } else if value <= u8::MAX as u64 {
            output.write(major_type | ADDITIONAL_INFO_1_BYTE);
            output.add(1).write(value as u8);
            2
        } else if value <= u16::MAX as u64 {
            output.write(major_type | ADDITIONAL_INFO_2_BYTES);
            output
                .add(1)
                .cast::<u16>()
                .write_unaligned((value as u16).to_be());
            3
        } else {
            encode_wide_argument_at(output, major_type, value)
        }
    }
}

/// Encodes an argument wider than 16 bits.
#[inline(always)]
unsafe fn encode_wide_argument_at(output: *mut u8, major_type: u8, value: u64) -> usize {
    unsafe {
        if value <= u32::MAX as u64 {
            output.write(major_type | ADDITIONAL_INFO_4_BYTES);
            output
                .add(1)
                .cast::<u32>()
                .write_unaligned((value as u32).to_be());
            5
        } else {
            output.write(major_type | ADDITIONAL_INFO_8_BYTES);
            output.add(1).cast::<u64>().write_unaligned(value.to_be());
            9
        }
    }
}

/// Encodes a non-negative integer as CBOR major type 0.
#[inline(always)]
unsafe fn encode_unsigned_at(output: *mut u8, value: u64) -> usize {
    unsafe { encode_argument_at(output, MAJOR_TYPE_UNSIGNED_INT, value) }
}

/// Encodes a negative integer as major type 1, which stores `-1 - value`.
#[inline(always)]
unsafe fn encode_negative_at(output: *mut u8, value: i64) -> usize {
    // Negative, so `-1 - value` cannot overflow.
    encode_negative_argument_at(output, (-1 - value) as u64)
}

/// Writes a major type 1 head for an argument the format stores directly.
///
/// The argument is the whole `u64` range, not the `0..=i64::MAX` an `i64` reaches,
/// because `u64::MAX` names `-2^64` and has no `i64`. The caller has reserved
/// `MAX_ENCODED_INT` writable bytes.
#[inline(always)]
fn encode_negative_argument_at(output: *mut u8, abs: u64) -> usize {
    // SAFETY: the caller provides room for the argument's head.
    unsafe { encode_argument_at(output, MAJOR_TYPE_NEGATIVE_INT, abs) }
}

/// Encodes a signed integer, choosing major type 0 or 1 from the sign.
///
/// CBOR has no signed type, and getting the split wrong decodes as the opposite
/// sign, so every signed writer funnels through here. `value` carries the bit
/// pattern of an `i64`, matching the `u64` the internal scalar writer takes.
#[inline(always)]
fn encode_signed_at(output: *mut u8, value: u64) -> usize {
    if value <= i64::MAX as u64 {
        // SAFETY: forwarding the caller's buffer guarantee.
        unsafe { encode_unsigned_at(output, value) }
    } else {
        // SAFETY: as above. Reinterpreting gives the intended negative value.
        unsafe { encode_negative_at(output, value as i64) }
    }
}

/// The number of bytes a length head occupies for a payload of `len` bytes.
#[inline(always)]
pub(crate) const fn head_len_for(len: usize) -> usize {
    if len <= 23 {
        1
    } else if len <= u8::MAX as usize {
        2
    } else if len <= u16::MAX as usize {
        3
    } else if len <= u32::MAX as usize {
        5
    } else {
        9
    }
}

/// Decodes an IEEE 754 `binary16` to `f32`.
///
/// Exact: every `binary16` is representable in an `f32`, so nothing is rounded
/// and a subnormal is not flushed to zero.
pub fn decode_f16(bits: u16) -> f32 {
    const SIGN_MASK: u16 = 0x8000;
    const EXP_MASK: u16 = 0x7c00;
    const MANT_MASK: u16 = 0x03ff;

    let sign = u32::from(bits & SIGN_MASK) << 16;
    let exp = u32::from(bits & EXP_MASK) >> 10;
    let mant = u32::from(bits & MANT_MASK);

    let f32_bits = if exp == 0 {
        if mant == 0 {
            // A signed zero.
            sign
        } else {
            // A subnormal is `mant * 2^-24`, renormalized into a normal `f32`.
            // Exact: the mantissa shifts left to fill 23 bits and the exponent
            // absorbs the difference. `k` is the highest set bit, so the value
            // lies in `[2^(k-24), 2^(k-23))` and the exponent is `k - 24`.
            let k = 31 - mant.leading_zeros();
            let f32_exp = (k as i32 - 24 + 127) as u32;
            let f32_mant = (mant << (23 - k)) & 0x007f_ffff;
            sign | (f32_exp << 23) | f32_mant
        }
    } else if exp == 0x1f {
        // Infinity, or a NaN with its payload shifted up into the f32 mantissa.
        sign | (0xff << 23) | (mant << 13)
    } else {
        sign | ((exp + 127 - 15) << 23) | (mant << 13)
    };

    f32::from_bits(f32_bits)
}

/// Encodes an `f32` as a `binary16`, rounding to nearest with ties to even.
///
/// Lossy in general: `binary16` has a smaller range and 10 mantissa bits. The
/// rounding is explicit rather than a truncating shift, so a value just below a
/// midpoint lands on the correct neighbour instead of being biased toward zero.
pub fn encode_f16(value: f32) -> u16 {
    const F32_EXP: u32 = 0x7f80_0000;
    const F32_MANT: u32 = 0x007f_ffff;

    let bits = value.to_bits();
    let sign = ((bits >> 16) as u16) & 0x8000;
    let exp = ((bits & F32_EXP) >> 23) as i32;
    let mant = bits & F32_MANT;

    if exp == 0xff {
        // Infinity stays. A NaN keeps its sign with the payload shifted down,
        // and the top payload bit is forced on so it never becomes an infinity.
        if mant == 0 {
            return sign | 0x7c00;
        }
        let payload = (mant >> 13) as u16;
        return sign | 0x7c00 | payload | u16::from(payload == 0);
    }

    if exp == 0 {
        // An f32 subnormal is below the smallest f16 subnormal (2^-24), so it
        // always rounds to a signed zero.
        return sign;
    }

    // Rebias from f32's exponent to f16's, folding in the implicit leading one.
    let half_exp = (exp - 127) + 15;
    let full_mant = mant | 0x0080_0000;

    if half_exp >= 0x1f {
        // Overflow, including at the midpoint to infinity.
        return sign | 0x7c00;
    }

    if half_exp >= 1 {
        // Normal. A subnormal covers `half_exp` -9 through 0, so 0 belongs to
        // the branch below and only 1 and up has a stored exponent.
        // `full_mant` holds 24 significant bits with the implicit leading one at
        // bit 23, so the stored 10-bit field is bits 22..=13.
        let mut half_mant = ((full_mant >> 13) & 0x03ff) as u16;
        let remainder = full_mant & 0x1fff;
        if remainder > 0x1000 || (remainder == 0x1000 && half_mant & 1 == 1) {
            half_mant += 1;
            if half_mant == 0x400 {
                // Carried into the exponent, possibly to infinity.
                return sign | (((half_exp + 1) as u16) << 10);
            }
        }
        sign | ((half_exp as u16) << 10) | half_mant
    } else if half_exp >= -10 {
        // Subnormal. The stored field is the significand shifted right by
        // `14 - half_exp`. It must stay below `0x400`; exactly `0x400` is the
        // carry into the smallest normal.
        let shift = 14 - half_exp;
        let mut half_mant = ((full_mant >> shift) & 0x03ff) as u16;
        let remainder = full_mant & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        if remainder > halfway || (remainder == halfway && half_mant & 1 == 1) {
            half_mant += 1;
            if half_mant == 0x400 {
                return sign | (1 << 10);
            }
        }
        sign | half_mant
    } else {
        // Below half the smallest subnormal, so a signed zero.
        sign
    }
}

/// Encoders for the homogeneous slice fast paths, one per scalar type.
///
/// Each writes a whole slice through one bounds check. A loop of scalar calls
/// re-checks the remaining capacity per element, which a caller that reserved
/// `MAX_ENCODED_INT` per element up front can skip.
macro_rules! impl_slice_writers {
    ($($method:ident, $scalar:ident, $ty:ty, $encode:expr;)*) => {
        $(
            #[doc = concat!("Writes a slice of `", stringify!($ty), "` values without an array header.")]
            #[inline]
            fn $method(&mut self, values: &[$ty]) -> Result<()> {
                if values.is_empty() {
                    return Ok(());
                }
                let Some(worst_case) = values.len().checked_mul(MAX_ENCODED_INT) else {
                    return Err(Error::BufferTooSmall);
                };
                if worst_case > self.buffer.len() - self.pos {
                    // Not enough room for the worst case: the per-element path
                    // needs only the bytes each value uses.
                    for &value in values {
                        self.$scalar(value)?;
                    }
                    return Ok(());
                }

                let output = unsafe { self.buffer.as_mut_ptr().add(self.pos) };
                let encode: fn(*mut u8, $ty) -> usize = $encode;
                let mut written = 0;
                for &value in values {
                    // SAFETY: the fast path reserved the worst-case length for
                    // every element, so each write stays in bounds.
                    written += unsafe { encode(output.add(written), value) };
                }
                self.pos += written;
                Ok(())
            }
        )*
    };
}

#[inline(always)]
fn encode_u8_at(output: *mut u8, value: u8) -> usize {
    // SAFETY: forwarding the caller's buffer guarantee.
    unsafe { encode_unsigned_at(output, value as u64) }
}

#[inline(always)]
fn encode_u16_at(output: *mut u8, value: u16) -> usize {
    // SAFETY: as above.
    unsafe { encode_unsigned_at(output, value as u64) }
}

#[inline(always)]
fn encode_u32_at(output: *mut u8, value: u32) -> usize {
    // SAFETY: as above.
    unsafe { encode_unsigned_at(output, value as u64) }
}

#[inline(always)]
fn encode_u64_at(output: *mut u8, value: u64) -> usize {
    // Wide arguments are common in u64 arrays. Test them before the compact
    // forms, while the narrower scalar encoders keep their small-value path.
    // SAFETY: forwarding the caller's buffer guarantee.
    unsafe {
        if value > u16::MAX as u64 {
            encode_wide_argument_at(output, MAJOR_TYPE_UNSIGNED_INT, value)
        } else {
            encode_unsigned_at(output, value)
        }
    }
}

#[inline(always)]
fn encode_i8_at(output: *mut u8, value: i8) -> usize {
    encode_signed_at(output, value as i64 as u64)
}

#[inline(always)]
fn encode_i16_at(output: *mut u8, value: i16) -> usize {
    encode_signed_at(output, value as i64 as u64)
}

#[inline(always)]
fn encode_i32_at(output: *mut u8, value: i32) -> usize {
    encode_signed_at(output, value as i64 as u64)
}

#[inline(always)]
fn encode_i64_at(output: *mut u8, value: i64) -> usize {
    encode_signed_at(output, value as u64)
}

/// Adapts a `f32` encoder to the `u64`-valued the internal scalar writer interface.
#[inline(always)]
fn encode_f32_int_at(output: *mut u8, value: u64) -> usize {
    encode_f32_at(output, f32::from_bits(value as u32))
}

#[inline(always)]
fn encode_f32_at(output: *mut u8, value: f32) -> usize {
    unsafe {
        output.write(FLOAT32_MARKER);
        output
            .add(1)
            .cast::<u32>()
            .write_unaligned(value.to_bits().to_be());
    }
    5
}

#[inline(always)]
fn encode_f64_int_at(output: *mut u8, value: u64) -> usize {
    encode_f64_at(output, f64::from_bits(value))
}

#[inline(always)]
fn encode_f64_at(output: *mut u8, value: f64) -> usize {
    unsafe {
        output.write(FLOAT64_MARKER);
        output
            .add(1)
            .cast::<u64>()
            .write_unaligned(value.to_bits().to_be());
    }
    9
}

// Adapters from the `u64`-valued the scalar writer to the typed encoders. The bit
// pattern is preserved both ways, so nothing is lost.
#[inline(always)]
fn encode_u8_int_at(output: *mut u8, value: u64) -> usize {
    encode_u8_at(output, value as u8)
}

#[inline(always)]
fn encode_u16_int_at(output: *mut u8, value: u64) -> usize {
    encode_u16_at(output, value as u16)
}

#[inline(always)]
fn encode_u32_int_at(output: *mut u8, value: u64) -> usize {
    encode_u32_at(output, value as u32)
}

#[inline(always)]
fn encode_u64_int_at(output: *mut u8, value: u64) -> usize {
    encode_u64_at(output, value)
}

#[inline(always)]
fn encode_i8_int_at(output: *mut u8, value: u64) -> usize {
    encode_i8_at(output, value as u8 as i8)
}

#[inline(always)]
fn encode_i16_int_at(output: *mut u8, value: u64) -> usize {
    encode_i16_at(output, value as u16 as i16)
}

#[inline(always)]
fn encode_i32_int_at(output: *mut u8, value: u64) -> usize {
    encode_i32_at(output, value as u32 as i32)
}

#[inline(always)]
fn encode_i64_int_at(output: *mut u8, value: u64) -> usize {
    encode_i64_at(output, value as i64)
}

#[inline(always)]
fn encode_f16_int_at(output: *mut u8, value: u64) -> usize {
    unsafe {
        output.write(FLOAT16_MARKER);
        // CBOR is big-endian, so the half-float payload has to be byte-swapped
        // on a little-endian target.
        output
            .add(1)
            .cast::<u16>()
            .write_unaligned((value as u16).to_be());
    }
    3
}

#[inline(always)]
fn encode_bool_at(output: *mut u8, value: bool) -> usize {
    // SAFETY: a single byte write at the caller's position.
    unsafe {
        output.write(if value {
            SIMPLE_VALUE_TRUE
        } else {
            SIMPLE_VALUE_FALSE
        });
    }
    1
}

macro_rules! impl_scalar_writers {
    () => {
        #[inline(always)]
        fn write_null(&mut self) -> Result<()> {
            self.write_bytes(&[SIMPLE_VALUE_NULL])
        }

        #[inline(always)]
        fn write_boolean(&mut self, value: bool) -> Result<()> {
            self.write_bytes(&[if value {
                SIMPLE_VALUE_TRUE
            } else {
                SIMPLE_VALUE_FALSE
            }])
        }

        #[inline(always)]
        fn write_simple_value(&mut self, value: u8) -> Result<()> {
            match value {
                0..=23 => self.write_bytes(&[MAJOR_TYPE_SIMPLE_FLOAT | value]),
                // 24 through 27 are the float widths, 28 through 30 are
                // unassigned and 31 is the break: none is a simple value.
                24..=31 => {
                    cold_path();
                    Err(Error::InvalidSimpleValue(value))
                }
                _ => self.write_bytes(&[MAJOR_TYPE_SIMPLE_FLOAT | ADDITIONAL_INFO_1_BYTE, value]),
            }
        }

        #[inline(always)]
        fn write_undefined(&mut self) -> Result<()> {
            self.write_bytes(&[SIMPLE_VALUE_UNDEFINED])
        }

        #[inline(always)]
        fn write_u8(&mut self, value: u8) -> Result<()> {
            self.write_int(value as u64, encode_u8_int_at)
        }

        #[inline(always)]
        fn write_u16(&mut self, value: u16) -> Result<()> {
            self.write_int(value as u64, encode_u16_int_at)
        }

        #[inline(always)]
        fn write_u32(&mut self, value: u32) -> Result<()> {
            self.write_int(value as u64, encode_u32_int_at)
        }

        #[inline(always)]
        fn write_u64(&mut self, value: u64) -> Result<()> {
            self.write_int(value, encode_u64_int_at)
        }

        #[inline(always)]
        fn write_negative(&mut self, argument: u64) -> Result<()> {
            self.write_int(argument, encode_negative_argument_at)
        }

        #[inline(always)]
        fn write_i8(&mut self, value: i8) -> Result<()> {
            self.write_int(value as i64 as u64, encode_i8_int_at)
        }

        #[inline(always)]
        fn write_i16(&mut self, value: i16) -> Result<()> {
            self.write_int(value as i64 as u64, encode_i16_int_at)
        }

        #[inline(always)]
        fn write_i32(&mut self, value: i32) -> Result<()> {
            self.write_int(value as i64 as u64, encode_i32_int_at)
        }

        #[inline(always)]
        fn write_i64(&mut self, value: i64) -> Result<()> {
            self.write_int(value as u64, encode_i64_int_at)
        }

        #[inline(always)]
        fn write_f16(&mut self, value: f32) -> Result<()> {
            self.write_int(encode_f16(value) as u64, encode_f16_int_at)
        }

        #[inline(always)]
        fn write_f32(&mut self, value: f32) -> Result<()> {
            self.write_int(value.to_bits() as u64, encode_f32_int_at)
        }

        #[inline(always)]
        fn write_f64(&mut self, value: f64) -> Result<()> {
            self.write_int(value.to_bits(), encode_f64_int_at)
        }
    };
}

/// A trait for writing CBOR-encoded data.
///
/// ## Examples
///
/// ```rust
/// use zerocbor::{Result, ToCbor, Write};
///
/// struct Point {
///     x: i32,
///     y: i32,
/// }
///
/// impl ToCbor for Point {
///     fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
///         writer.write_array_len(2)?;
///         writer.write_i32(self.x)?;
///         writer.write_i32(self.y)?;
///         Ok(())
///     }
/// }
/// ```
///
/// Raw pointer encoders are internal implementation details, unavailable to
/// safe custom serializers:
///
/// ```compile_fail
/// fn raw_encode<W: zerocbor::Write>(writer: &mut W) {
///     writer.write_int(0, |_, _| usize::MAX).unwrap();
/// }
/// ```
pub trait Write {
    /// Appends already-encoded bytes.
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()>;

    /// Writes a null value.
    fn write_null(&mut self) -> Result<()>;

    /// Writes a boolean value.
    fn write_boolean(&mut self, b: bool) -> Result<()>;

    /// Writes a simple value that is not one of the three float widths.
    ///
    /// Accepts 0 through 23, which fit in the initial byte, and 32 through 255,
    /// the two-byte form of RFC 8949 Section 3.3. 24 through 31 are an error:
    /// three are the float widths and the rest unassigned.
    fn write_simple_value(&mut self, value: u8) -> Result<()>;

    /// Writes `undefined`.
    fn write_undefined(&mut self) -> Result<()>;

    /// Writes an unsigned 8-bit integer.
    fn write_u8(&mut self, u: u8) -> Result<()>;

    /// Writes an unsigned 16-bit integer.
    fn write_u16(&mut self, u: u16) -> Result<()>;

    /// Writes an unsigned 32-bit integer.
    fn write_u32(&mut self, u: u32) -> Result<()>;

    /// Writes an unsigned 64-bit integer.
    fn write_u64(&mut self, u: u64) -> Result<()>;

    /// Writes a major type 1 head for a negative integer.
    ///
    /// `argument` is the negation the format stores: `-1` is `0`, `-2^64` is
    /// `u64::MAX`. A `u64` rather than an `i64` because the whole range is
    /// meaningful and `-2^64` has no `i64`.
    fn write_negative(&mut self, argument: u64) -> Result<()>;

    /// Writes a signed 8-bit integer.
    fn write_i8(&mut self, i: i8) -> Result<()>;

    /// Writes a signed 16-bit integer.
    fn write_i16(&mut self, i: i16) -> Result<()>;

    /// Writes a signed 32-bit integer.
    fn write_i32(&mut self, i: i32) -> Result<()>;

    /// Writes a signed 64-bit integer.
    fn write_i64(&mut self, i: i64) -> Result<()>;

    /// Writes a 16-bit floating-point number.
    fn write_f16(&mut self, f: f32) -> Result<()>;

    /// Writes a 32-bit floating-point number.
    fn write_f32(&mut self, f: f32) -> Result<()>;

    /// Writes a 64-bit floating-point number.
    fn write_f64(&mut self, f: f64) -> Result<()>;

    /// Writes a slice of booleans without an array header. Override for a bulk
    /// encode.
    #[inline(always)]
    fn write_boolean_slice(&mut self, values: &[bool]) -> Result<()> {
        for &value in values {
            self.write_boolean(value)?;
        }
        Ok(())
    }

    /// Writes a slice of unsigned 8-bit integers without an array header.
    #[inline(always)]
    fn write_u8_slice(&mut self, values: &[u8]) -> Result<()> {
        for &value in values {
            self.write_u8(value)?;
        }
        Ok(())
    }

    /// Writes a slice of unsigned 16-bit integers without an array header.
    #[inline(always)]
    fn write_u16_slice(&mut self, values: &[u16]) -> Result<()> {
        for &value in values {
            self.write_u16(value)?;
        }
        Ok(())
    }

    /// Writes a slice of unsigned 32-bit integers without an array header.
    #[inline(always)]
    fn write_u32_slice(&mut self, values: &[u32]) -> Result<()> {
        for &value in values {
            self.write_u32(value)?;
        }
        Ok(())
    }

    /// Writes a slice of unsigned 64-bit integers without an array header.
    #[inline(always)]
    fn write_u64_slice(&mut self, values: &[u64]) -> Result<()> {
        for &value in values {
            self.write_u64(value)?;
        }
        Ok(())
    }

    /// Writes a slice of signed 8-bit integers without an array header.
    #[inline(always)]
    fn write_i8_slice(&mut self, values: &[i8]) -> Result<()> {
        for &value in values {
            self.write_i8(value)?;
        }
        Ok(())
    }

    /// Writes a slice of signed 16-bit integers without an array header.
    #[inline(always)]
    fn write_i16_slice(&mut self, values: &[i16]) -> Result<()> {
        for &value in values {
            self.write_i16(value)?;
        }
        Ok(())
    }

    /// Writes a slice of signed 32-bit integers without an array header.
    #[inline(always)]
    fn write_i32_slice(&mut self, values: &[i32]) -> Result<()> {
        for &value in values {
            self.write_i32(value)?;
        }
        Ok(())
    }

    /// Writes a slice of signed 64-bit integers without an array header.
    #[inline(always)]
    fn write_i64_slice(&mut self, values: &[i64]) -> Result<()> {
        for &value in values {
            self.write_i64(value)?;
        }
        Ok(())
    }

    /// Writes a slice of 32-bit floats without an array header.
    #[inline(always)]
    fn write_f32_slice(&mut self, values: &[f32]) -> Result<()> {
        for &value in values {
            self.write_f32(value)?;
        }
        Ok(())
    }

    /// Writes a slice of 64-bit floats without an array header.
    #[inline(always)]
    fn write_f64_slice(&mut self, values: &[f64]) -> Result<()> {
        for &value in values {
            self.write_f64(value)?;
        }
        Ok(())
    }

    /// Writes a UTF-8 string as a CBOR text string.
    fn write_string(&mut self, s: &str) -> Result<()>;

    /// Writes a byte slice as a CBOR byte string.
    fn write_binary(&mut self, data: &[u8]) -> Result<()>;

    /// Writes a tag head.
    fn write_tag(&mut self, tag: u64) -> Result<()>;

    /// Writes an array header with the length.
    fn write_array_len(&mut self, len: usize) -> Result<()>;

    /// Writes a map header with the length.
    fn write_map_len(&mut self, len: usize) -> Result<()>;
}

/// Writes into a caller-provided byte slice.
///
/// Every write is bounds-checked, independently of user-provided size hints.
pub(crate) struct SliceWriter<'a> {
    buffer: &'a mut [u8],
    pos: usize,
}

impl<'a> SliceWriter<'a> {
    #[inline(always)]
    fn write_int(&mut self, value: u64, encode: fn(*mut u8, u64) -> usize) -> Result<()> {
        if MAX_ENCODED_INT > self.buffer.len() - self.pos {
            return self.write_int_narrow(value, encode);
        }
        // SAFETY: the bounds check guarantees nine writable bytes. Only
        // internal encoders, which initialize their returned length, reach here.
        let written = unsafe { encode(self.buffer.as_mut_ptr().add(self.pos), value) };
        self.pos += written;
        Ok(())
    }

    /// Creates a bounds-checked writer over `buffer`.
    pub fn new(buffer: &'a mut [u8]) -> Self {
        SliceWriter { buffer, pos: 0 }
    }
}

impl<'a> SliceWriter<'a> {
    /// Returns the number of bytes written so far.
    #[inline(always)]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Encodes into a stack temporary, then copies only what was produced.
    ///
    /// Taken when the remaining buffer is under `MAX_ENCODED_INT`: one extra
    /// copy, but an exactly sized destination works, which is the common case
    /// for a stack buffer.
    #[cold]
    #[inline(never)]
    fn write_int_narrow(&mut self, value: u64, encode: fn(*mut u8, u64) -> usize) -> Result<()> {
        let mut tmp = [0u8; MAX_ENCODED_INT];
        // SAFETY: `tmp` is `MAX_ENCODED_INT` and no encoder writes more.
        let written = encode(tmp.as_mut_ptr(), value);
        self.write_bytes(&tmp[..written])
    }

    /// Writes the head directly into the destination, then copies the payload.
    #[inline(always)]
    fn write_payload(&mut self, major: u8, payload: &[u8]) -> Result<()> {
        let head_len = head_len_for(payload.len());
        if head_len > self.buffer.len() - self.pos {
            return Err(Error::BufferTooSmall);
        }
        // SAFETY: the check above covers the actual head.
        let written = unsafe {
            encode_argument_at(
                self.buffer.as_mut_ptr().add(self.pos),
                major,
                payload.len() as u64,
            )
        };
        self.pos += written;
        self.write_bytes(payload)
    }

    /// Reserves `len` writable bytes and returns them.
    #[inline(always)]
    fn take_slice(&mut self, len: usize) -> Result<&mut [u8]> {
        if len > self.buffer.len() - self.pos {
            cold_path();
            return Err(Error::BufferTooSmall);
        }
        // SAFETY: as above; `[u8]` is aligned to 1.
        let slice = unsafe { self.buffer.get_unchecked_mut(self.pos..self.pos + len) };
        self.pos += len;
        Ok(slice)
    }
}

impl<'a> Write for SliceWriter<'a> {
    #[inline(always)]
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.take_slice(bytes.len())?.copy_from_slice(bytes);
        Ok(())
    }

    impl_scalar_writers!();

    impl_slice_writers! {
        write_boolean_slice, write_boolean, bool, encode_bool_at;
        write_u8_slice, write_u8, u8, encode_u8_at;
        write_u16_slice, write_u16, u16, encode_u16_at;
        write_u32_slice, write_u32, u32, encode_u32_at;
        write_u64_slice, write_u64, u64, encode_u64_at;
        write_i8_slice, write_i8, i8, encode_i8_at;
        write_i16_slice, write_i16, i16, encode_i16_at;
        write_i32_slice, write_i32, i32, encode_i32_at;
        write_i64_slice, write_i64, i64, encode_i64_at;
        write_f32_slice, write_f32, f32, encode_f32_at;
        write_f64_slice, write_f64, f64, encode_f64_at;
    }

    #[inline(always)]
    fn write_string(&mut self, s: &str) -> Result<()> {
        self.write_payload(MAJOR_TYPE_TEXT_STRING, s.as_bytes())
    }

    #[inline(always)]
    fn write_binary(&mut self, data: &[u8]) -> Result<()> {
        self.write_payload(MAJOR_TYPE_BYTE_STRING, data)
    }

    #[inline(always)]
    fn write_tag(&mut self, tag: u64) -> Result<()> {
        write_tag(self, tag)
    }

    #[inline(always)]
    fn write_array_len(&mut self, len: usize) -> Result<()> {
        write_head(self, MAJOR_TYPE_ARRAY, len)
    }

    #[inline(always)]
    fn write_map_len(&mut self, len: usize) -> Result<()> {
        write_head(self, MAJOR_TYPE_MAP, len)
    }
}

/// Writes a length head followed by a payload.
#[inline(always)]
fn write_head_and_payload<W: Write>(writer: &mut W, major_type: u8, payload: &[u8]) -> Result<()> {
    let mut head = [0u8; MAX_ENCODED_INT];
    // SAFETY: `head` is `MAX_ENCODED_INT` bytes, which is the most a length
    // head can occupy.
    let head_len =
        unsafe { encode_argument_at(head.as_mut_ptr(), major_type, payload.len() as u64) };
    debug_assert_eq!(head_len, head_len_for(payload.len()));
    writer.write_bytes(&head[..head_len])?;
    writer.write_bytes(payload)
}

/// Writes into a growable `Vec<u8>`.
pub(crate) struct VecWriter {
    buffer: Vec<u8>,
}

impl Default for VecWriter {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl VecWriter {
    #[inline(always)]
    fn write_int(&mut self, value: u64, encode: fn(*mut u8, u64) -> usize) -> Result<()> {
        self.buffer.reserve(MAX_ENCODED_INT);
        let start = self.buffer.len();
        // SAFETY: the reserve guarantees `MAX_ENCODED_INT` writable bytes, and
        // every encoder writes at most that many.
        let written = unsafe { encode(self.buffer.as_mut_ptr().add(start), value) };
        // SAFETY: as in `write_slice_values`.
        unsafe { self.buffer.set_len(start + written) };
        Ok(())
    }

    /// Creates an empty writer.
    pub fn new() -> Self {
        VecWriter { buffer: Vec::new() }
    }

    /// Creates a writer that tries to reserve `capacity` bytes up front.
    ///
    /// A hint is only an optimization, so a failed reservation is ignored
    /// rather than aborting the serialization before any output actually
    /// requires the memory.
    pub fn with_capacity_hint(capacity: usize) -> Self {
        let mut buffer = Vec::new();
        let _ = buffer.try_reserve(capacity);
        VecWriter { buffer }
    }

    /// Consumes the writer, returning the encoded bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.buffer
    }

    /// Encodes a whole slice in one pass.
    ///
    /// Reserving once and advancing a local offset avoids both the per-push
    /// capacity check and the per-element bookkeeping of `extend_from_slice`.
    /// The length is committed once at the end, so a panic mid-encode cannot
    /// leave a half-initialized length behind.
    #[inline]
    fn write_slice_values<T, E>(&mut self, values: &[T], encode: E) -> Result<()>
    where
        T: Copy,
        E: Fn(*mut u8, T) -> usize,
    {
        if values.is_empty() {
            return Ok(());
        }
        let Some(worst_case) = values.len().checked_mul(MAX_ENCODED_INT) else {
            return Err(Error::BufferTooSmall);
        };
        self.buffer.reserve(worst_case);

        let start = self.buffer.len();
        let output = unsafe { self.buffer.as_mut_ptr().add(start) };
        let mut written = 0;
        for value in values {
            // SAFETY: the reserve covers the worst-case length for every
            // element, so no write can run past the allocation.
            written += unsafe { encode(output.add(written), *value) };
        }
        // SAFETY: `written` bytes at `start` were just initialized, and
        // `start + written` is at most `buffer.capacity()` by the reserve.
        unsafe { self.buffer.set_len(start + written) };
        Ok(())
    }
}

macro_rules! impl_vec_slice_writers {
    () => {
        #[inline(always)]
        fn write_boolean_slice(&mut self, values: &[bool]) -> Result<()> {
            self.write_slice_values(values, encode_bool_at)
        }

        #[inline(always)]
        fn write_u8_slice(&mut self, values: &[u8]) -> Result<()> {
            self.write_slice_values(values, encode_u8_at)
        }

        #[inline(always)]
        fn write_u16_slice(&mut self, values: &[u16]) -> Result<()> {
            self.write_slice_values(values, encode_u16_at)
        }

        #[inline(always)]
        fn write_u32_slice(&mut self, values: &[u32]) -> Result<()> {
            self.write_slice_values(values, encode_u32_at)
        }

        #[inline(always)]
        fn write_u64_slice(&mut self, values: &[u64]) -> Result<()> {
            self.write_slice_values(values, encode_u64_at)
        }

        #[inline(always)]
        fn write_i8_slice(&mut self, values: &[i8]) -> Result<()> {
            self.write_slice_values(values, encode_i8_at)
        }

        #[inline(always)]
        fn write_i16_slice(&mut self, values: &[i16]) -> Result<()> {
            self.write_slice_values(values, encode_i16_at)
        }

        #[inline(always)]
        fn write_i32_slice(&mut self, values: &[i32]) -> Result<()> {
            self.write_slice_values(values, encode_i32_at)
        }

        #[inline(always)]
        fn write_i64_slice(&mut self, values: &[i64]) -> Result<()> {
            self.write_slice_values(values, encode_i64_at)
        }

        #[inline(always)]
        fn write_f32_slice(&mut self, values: &[f32]) -> Result<()> {
            self.write_slice_values(values, encode_f32_at)
        }

        #[inline(always)]
        fn write_f64_slice(&mut self, values: &[f64]) -> Result<()> {
            self.write_slice_values(values, encode_f64_at)
        }
    };
}

impl Write for VecWriter {
    #[inline(always)]
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    impl_scalar_writers!();

    impl_vec_slice_writers!();

    #[inline(always)]
    fn write_string(&mut self, s: &str) -> Result<()> {
        write_head_and_payload(self, MAJOR_TYPE_TEXT_STRING, s.as_bytes())
    }

    #[inline(always)]
    fn write_binary(&mut self, data: &[u8]) -> Result<()> {
        write_head_and_payload(self, MAJOR_TYPE_BYTE_STRING, data)
    }

    #[inline(always)]
    fn write_tag(&mut self, tag: u64) -> Result<()> {
        write_tag(self, tag)
    }

    #[inline(always)]
    fn write_array_len(&mut self, len: usize) -> Result<()> {
        write_head(self, MAJOR_TYPE_ARRAY, len)
    }

    #[inline(always)]
    fn write_map_len(&mut self, len: usize) -> Result<()> {
        write_head(self, MAJOR_TYPE_MAP, len)
    }
}

/// Writes to a [`std::io::Write`].
#[cfg(feature = "std")]
pub(crate) struct IOWriter<W: std::io::Write> {
    writer: W,
}

#[cfg(feature = "std")]
impl<W: std::io::Write> IOWriter<W> {
    #[inline(always)]
    fn write_int(&mut self, value: u64, encode: fn(*mut u8, u64) -> usize) -> Result<()> {
        let mut buf = [0u8; MAX_ENCODED_INT];
        let written = encode(buf.as_mut_ptr(), value);
        self.write_bytes(&buf[..written])
    }

    /// Wraps `writer`.
    pub fn new(writer: W) -> Self {
        IOWriter { writer }
    }
}

#[cfg(feature = "std")]
impl<W: std::io::Write> Write for IOWriter<W> {
    #[inline(always)]
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes).map_err(Error::IoError)
    }

    impl_scalar_writers!();

    #[inline(always)]
    fn write_boolean_slice(&mut self, values: &[bool]) -> Result<()> {
        let mut buf = [0u8; 256];
        for chunk in values.chunks(buf.len()) {
            for (slot, &value) in buf.iter_mut().zip(chunk) {
                *slot = if value {
                    SIMPLE_VALUE_TRUE
                } else {
                    SIMPLE_VALUE_FALSE
                };
            }
            self.write_bytes(&buf[..chunk.len()])?;
        }
        Ok(())
    }

    #[inline(always)]
    fn write_string(&mut self, s: &str) -> Result<()> {
        write_head_and_payload(self, MAJOR_TYPE_TEXT_STRING, s.as_bytes())
    }

    #[inline(always)]
    fn write_binary(&mut self, data: &[u8]) -> Result<()> {
        write_head_and_payload(self, MAJOR_TYPE_BYTE_STRING, data)
    }

    #[inline(always)]
    fn write_tag(&mut self, tag: u64) -> Result<()> {
        write_tag(self, tag)
    }

    #[inline(always)]
    fn write_array_len(&mut self, len: usize) -> Result<()> {
        write_head(self, MAJOR_TYPE_ARRAY, len)
    }

    #[inline(always)]
    fn write_map_len(&mut self, len: usize) -> Result<()> {
        write_head(self, MAJOR_TYPE_MAP, len)
    }
}

/// Writes a tag head.
#[inline(always)]
fn write_tag<W: Write>(writer: &mut W, tag: u64) -> Result<()> {
    let mut head = [0u8; MAX_ENCODED_INT];
    // SAFETY: `head` is `MAX_ENCODED_INT` bytes, which bounds any tag head.
    let head_len = unsafe { encode_argument_at(head.as_mut_ptr(), MAJOR_TYPE_TAG, tag) };
    writer.write_bytes(&head[..head_len])
}

/// Writes a length head with the given major type.
#[inline(always)]
fn write_head<W: Write>(writer: &mut W, major_type: u8, len: usize) -> Result<()> {
    let mut head = [0u8; MAX_ENCODED_INT];
    // SAFETY: `head` is `MAX_ENCODED_INT` bytes, which bounds any length head.
    let head_len = unsafe { encode_argument_at(head.as_mut_ptr(), major_type, len as u64) };
    writer.write_bytes(&head[..head_len])
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    /// The bulk encoders must produce exactly what the scalar path produces.
    #[track_caller]
    fn check_slice<T, F, S>(values: &[T], bulk: F, scalar: S)
    where
        T: Copy,
        F: Fn(&mut SliceWriter<'_>, &[T]) -> Result<()>,
        S: Fn(&mut SliceWriter<'_>, T) -> Result<()>,
    {
        let mut expected = vec![0u8; values.len() * MAX_ENCODED_INT + 1];
        let written = {
            let mut writer = SliceWriter::new(&mut expected);
            for &value in values {
                scalar(&mut writer, value).unwrap();
            }
            writer.position()
        };
        let expected = expected[..written].to_vec();

        // A buffer sized exactly to the scalar output forces the bulk writer
        // onto its fallback path, since that path reserves the worst case.
        let mut exact = vec![0u8; expected.len()];
        let written = {
            let mut writer = SliceWriter::new(&mut exact);
            bulk(&mut writer, values).unwrap();
            writer.position()
        };
        assert_eq!(&exact[..written], &expected[..], "fallback path differs");

        // A roomy buffer exercises the bulk fast path.
        let mut roomy = vec![0u8; values.len() * MAX_ENCODED_INT + 1];
        let written = {
            let mut writer = SliceWriter::new(&mut roomy);
            bulk(&mut writer, values).unwrap();
            writer.position()
        };
        assert_eq!(&roomy[..written], &expected[..], "fast path differs");
    }

    #[test]
    fn bulk_slices_match_the_scalar_path() {
        macro_rules! check {
            ($ty:ty, $values:expr, $slice:ident, $scalar:ident) => {
                check_slice(
                    &$values,
                    |w, v: &[$ty]| w.$slice(v),
                    |w, v: $ty| w.$scalar(v),
                )
            };
        }

        check!(bool, [false, true], write_boolean_slice, write_boolean);
        check!(u8, [0u8, 23, 24, u8::MAX], write_u8_slice, write_u8);
        check!(
            u16,
            [0u16, 23, 24, 255, 256, u16::MAX],
            write_u16_slice,
            write_u16
        );
        check!(
            u32,
            [0u32, 23, 24, 255, 256, 65535, 65536, u32::MAX],
            write_u32_slice,
            write_u32
        );
        check!(
            u64,
            [
                0u64,
                23,
                24,
                255,
                256,
                65535,
                65536,
                u32::MAX as u64,
                u64::MAX
            ],
            write_u64_slice,
            write_u64
        );
        check!(i8, [i8::MIN, -24, -1, 0, i8::MAX], write_i8_slice, write_i8);
        check!(
            i16,
            [i16::MIN, -256, -24, -1, 0, i16::MAX],
            write_i16_slice,
            write_i16
        );
        check!(
            i32,
            [i32::MIN, -65536, -256, -24, -1, 0, i32::MAX],
            write_i32_slice,
            write_i32
        );
        check!(
            i64,
            [i64::MIN, -4294967296, -65536, -256, -24, -1, 0, 1, i64::MAX],
            write_i64_slice,
            write_i64
        );
        check!(
            f32,
            [0.0f32, -0.0, 1.0, f32::MIN, f32::MAX],
            write_f32_slice,
            write_f32
        );
        check!(
            f64,
            [0.0f64, -0.0, 1.0, f64::MIN, f64::MAX],
            write_f64_slice,
            write_f64
        );
    }
}
