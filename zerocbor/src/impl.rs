use crate::write::head_len_for as header_size;
use core::hint::cold_path;

use crate::consts::FLOAT16_MARKER;
use crate::consts::FLOAT32_MARKER;
use crate::consts::FLOAT64_MARKER;
use crate::{Error, FromCbor, Read, ToCbor, Write};
use alloc::string::ToString;

#[cfg(feature = "std")]
use core::hash::Hash;

#[inline]
fn sequence_size_hint<T: ToCbor>(len: usize) -> Option<crate::TrustedSizeHint> {
    let element = T::max_size()?.upper_bound();
    let size = element.checked_mul(len)?.checked_add(header_size(len))?;
    // SAFETY: the header is exact and every element is bounded by `element`.
    Some(unsafe { crate::TrustedSizeHint::new_unchecked(size) })
}

#[inline]
fn map_size_hint<K: ToCbor, V: ToCbor>(len: usize) -> Option<crate::TrustedSizeHint> {
    let pair = K::max_size()?
        .upper_bound()
        .checked_add(V::max_size()?.upper_bound())?;
    let header = header_size(len);
    let size = pair.checked_mul(len)?.checked_add(header)?;
    // SAFETY: the header is exact and every key/value pair is bounded by `pair`.
    Some(unsafe { crate::TrustedSizeHint::new_unchecked(size) })
}

#[inline]
pub(crate) fn string_size_hint(len: usize) -> Option<crate::TrustedSizeHint> {
    let size = len.checked_add(header_size(len))?;
    // SAFETY: CBOR string headers depend only on the byte length.
    Some(unsafe { crate::TrustedSizeHint::new_unchecked(size) })
}

macro_rules! impl_scalar {
    // Types with a bulk slice encoder.
    ($ty:ty, $write_fn:ident, $read_fn:ident, $size:expr, $max:expr, $write_slice_fn:ident) => {
        impl<'a> FromCbor<'a> for $ty {
            #[inline(always)]
            fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self> {
                reader.$read_fn()
            }
        }

        impl ToCbor for $ty {
            #[inline(always)]
            fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
                writer.$write_fn(*self)
            }

            #[inline(always)]
            fn write_slice<W: Write>(values: &[Self], writer: &mut W) -> crate::Result<()> {
                writer.$write_slice_fn(values)
            }

            #[inline(always)]
            fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
                // SAFETY: primitive encodings are completely determined by their value.
                Some(unsafe { crate::TrustedSizeHint::new_unchecked(($size)(*self)) })
            }

            #[inline(always)]
            fn max_size() -> Option<crate::TrustedSizeHint> {
                // SAFETY: this is the largest encoding emitted for this primitive type.
                Some(unsafe { crate::TrustedSizeHint::new_unchecked($max) })
            }
        }
    };
    // Types without one, which keep the trait's default per-element `write_slice`.
    ($ty:ty, $write_fn:ident, $read_fn:ident, $size:expr, $max:expr) => {
        impl<'a> FromCbor<'a> for $ty {
            #[inline(always)]
            fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
            where
                Self: Sized,
            {
                reader.$read_fn()
            }
        }

        impl ToCbor for $ty {
            #[inline(always)]
            fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
                writer.$write_fn(*self)
            }

            #[inline(always)]
            fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
                // SAFETY: primitive encodings are completely determined by their value.
                Some(unsafe { crate::TrustedSizeHint::new_unchecked(($size)(*self)) })
            }

            #[inline(always)]
            fn max_size() -> Option<crate::TrustedSizeHint> {
                // SAFETY: this is the largest encoding emitted for this primitive type.
                Some(unsafe { crate::TrustedSizeHint::new_unchecked($max) })
            }
        }
    };
}

impl_scalar!(bool, write_boolean, read_boolean, |_| 1, 1);
impl_scalar!(
    i8,
    write_i8,
    read_i8,
    |v: i8| if (-24..=-1).contains(&v) { 1 } else { 2 },
    2,
    write_i8_slice
);
impl_scalar!(
    i16,
    write_i16,
    read_i16,
    |v: i16| if (-24..=-1).contains(&v) {
        1
    } else if (-256..=-25).contains(&v) {
        2
    } else {
        3
    },
    3,
    write_i16_slice
);
impl_scalar!(
    i32,
    write_i32,
    read_i32,
    |v: i32| if (-24..=-1).contains(&v) {
        1
    } else if (-256..=-25).contains(&v) {
        2
    } else if (-65536..=-257).contains(&v) {
        3
    } else {
        5
    },
    5,
    write_i32_slice
);
impl_scalar!(
    i64,
    write_i64,
    read_i64,
    |v: i64| if (-24..=-1).contains(&v) {
        1
    } else if (-256..=-25).contains(&v) {
        2
    } else if (-65536..=-257).contains(&v) {
        3
    } else if (-4294967296..=-65537).contains(&v) {
        5
    } else {
        9
    },
    9,
    write_i64_slice
);
impl_scalar!(
    u8,
    write_u8,
    read_u8,
    |v: u8| if v <= 23 { 1 } else { 2 },
    2,
    write_u8_slice
);
impl_scalar!(
    u16,
    write_u16,
    read_u16,
    |v: u16| if v <= 23 {
        1
    } else if v <= 255 {
        2
    } else {
        3
    },
    3,
    write_u16_slice
);
impl_scalar!(
    u32,
    write_u32,
    read_u32,
    |v: u32| if v <= 23 {
        1
    } else if v <= 255 {
        2
    } else if v <= 65535 {
        3
    } else {
        5
    },
    5,
    write_u32_slice
);
impl_scalar!(
    u64,
    write_u64,
    read_u64,
    |v: u64| if v <= 23 {
        1
    } else if v <= 255 {
        2
    } else if v <= 65535 {
        3
    } else if v <= 4294967295 {
        5
    } else {
        9
    },
    9,
    write_u64_slice
);
/// Reads any CBOR float into an `f32`.
///
/// A value may arrive at any of the three widths, so a reader widens rather than
/// rejects. Only a non-float in a float's place is a type error.
fn read_f32_lenient<'a, R: Read<'a>>(reader: &mut R) -> crate::Result<f32> {
    let byte = reader.peek_initial_byte()?;
    if byte == FLOAT16_MARKER {
        reader.read_f16()
    } else if byte == FLOAT64_MARKER {
        reader.read_f64().map(|f| f as f32)
    } else {
        reader.read_f32()
    }
}

/// Reads any CBOR float into an `f64`, which can represent every width.
fn read_f64_lenient<'a, R: Read<'a>>(reader: &mut R) -> crate::Result<f64> {
    let byte = reader.peek_initial_byte()?;
    if byte == FLOAT16_MARKER {
        reader.read_f16().map(|f| f as f64)
    } else if byte == FLOAT32_MARKER {
        reader.read_f32().map(|f| f as f64)
    } else {
        reader.read_f64()
    }
}

/// Writes the one `NaN` encoding and reports whether the value was a `NaN`.
///
/// RFC 8949 Section 4.1 names `0xf97e00` and Section 4.2.1 requires the
/// preferred serialization, so a `NaN` goes out as those three bytes whatever
/// its mantissa holds. A payload would make the bytes a function of something
/// the format does not define, and since narrowing keeps only part of a wide
/// one, the same `NaN` from a `binary16` and from a `double` would reach one
/// document as two.
///
/// The explicit-width writers on [`Write`] are separate: a caller asking for a
/// `binary64` gets one, payload and all, because the width is the request.
#[inline(always)]
pub(crate) fn write_canonical_nan<W: Write>(is_nan: bool, writer: &mut W) -> crate::Result<bool> {
    if !is_nan {
        return Ok(false);
    }
    writer.write_bytes(&[FLOAT16_MARKER, 0x7e, 0x00])?;
    Ok(true)
}

pub(crate) fn preferred_float_marker(value: f64) -> u8 {
    if value.is_nan() {
        // No narrowing test can choose: a `NaN` is not equal to itself.
        return FLOAT16_MARKER;
    }
    // A value that survives a round trip through `f32` is exactly an `f32`, and
    // one of those may be exactly a `binary16`.
    let single = value as f32;
    if single as f64 == value {
        return if f16_preserves(single) {
            FLOAT16_MARKER
        } else {
            FLOAT32_MARKER
        };
    }
    FLOAT64_MARKER
}

/// Whether `value` is exactly a `binary16`, checked by re-encoding: the encoder
/// rounds to nearest, so a value that comes back different was not representable.
#[inline(always)]
pub(crate) fn f16_preserves(value: f32) -> bool {
    crate::write::decode_f16(crate::write::encode_f16(value)) == value
}

impl<'a> FromCbor<'a> for f32 {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self> {
        read_f32_lenient(reader)
    }
}

impl ToCbor for f32 {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        // Narrowest width that holds the value: a `f32` that is a `binary16`
        // goes out as three bytes.
        if write_canonical_nan(self.is_nan(), writer)? {
            return Ok(());
        }
        if f16_preserves(*self) {
            writer.write_f16(*self)
        } else {
            writer.write_f32(*self)
        }
    }

    #[inline(always)]
    fn write_slice<W: Write>(values: &[Self], writer: &mut W) -> crate::Result<()> {
        for value in values {
            if write_canonical_nan(value.is_nan(), writer)? {
                continue;
            }
            if f16_preserves(*value) {
                writer.write_f16(*value)?;
            } else {
                writer.write_f32(*value)?;
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        // SAFETY: the width is decided by the same test the writer uses, and a
        // `binary16` is three bytes and a `binary32` is five.
        Some(unsafe {
            crate::TrustedSizeHint::new_unchecked(if f16_preserves(*self) { 3 } else { 5 })
        })
    }

    #[inline(always)]
    fn max_size() -> Option<crate::TrustedSizeHint> {
        // SAFETY: as above, and five is the wider of the two.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(5) })
    }
}

impl<'a> FromCbor<'a> for f64 {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self> {
        read_f64_lenient(reader)
    }
}

/// Half-precision floats, behind the `f16` feature. The type is the language's
/// own, so this only adds the two trait impls.
#[cfg(feature = "f16")]
impl<'a> FromCbor<'a> for f16 {
    /// Reads any of the three float widths, narrowing to `binary16`.
    ///
    /// A wider input is accepted, since a producer with no half-precision type
    /// writes a single or a double. Narrowing is one step from the wire width,
    /// so a `float 32` rounds once rather than twice.
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self> {
        match reader.peek_initial_byte()? {
            FLOAT16_MARKER => Ok(reader.read_f16()? as f16),
            FLOAT32_MARKER => Ok(reader.read_f32()? as f16),
            _ => Ok(reader.read_f64()? as f16),
        }
    }
}

#[cfg(feature = "f16")]
impl ToCbor for f16 {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        if write_canonical_nan((*self as f32).is_nan(), writer)? {
            return Ok(());
        }
        writer.write_f16(*self as f32)
    }

    #[inline(always)]
    fn write_slice<W: Write>(values: &[Self], writer: &mut W) -> crate::Result<()> {
        for value in values {
            if write_canonical_nan((*value as f32).is_nan(), writer)? {
                continue;
            }
            writer.write_f16(*value as f32)?;
        }
        Ok(())
    }

    #[inline(always)]
    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        // SAFETY: an `f16` always encodes to exactly three bytes.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(3) })
    }

    #[inline(always)]
    fn max_size() -> Option<crate::TrustedSizeHint> {
        // SAFETY: as above.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(3) })
    }
}

impl ToCbor for f64 {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        if write_canonical_nan(self.is_nan(), writer)? {
            return Ok(());
        }
        match preferred_float_marker(*self) {
            FLOAT16_MARKER => writer.write_f16(*self as f32),
            FLOAT32_MARKER => writer.write_f32(*self as f32),
            _ => writer.write_f64(*self),
        }
    }

    #[inline(always)]
    fn write_slice<W: Write>(values: &[Self], writer: &mut W) -> crate::Result<()> {
        for value in values {
            if write_canonical_nan(value.is_nan(), writer)? {
                continue;
            }
            match preferred_float_marker(*value) {
                FLOAT16_MARKER => writer.write_f16(*value as f32)?,
                FLOAT32_MARKER => writer.write_f32(*value as f32)?,
                _ => writer.write_f64(*value)?,
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        // SAFETY: the width is decided by the same test the writer uses.
        Some(unsafe {
            crate::TrustedSizeHint::new_unchecked(match preferred_float_marker(*self) {
                FLOAT16_MARKER => 3,
                FLOAT32_MARKER => 5,
                _ => 9,
            })
        })
    }

    #[inline(always)]
    fn max_size() -> Option<crate::TrustedSizeHint> {
        // SAFETY: as above.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(9) })
    }
}

impl<'a> FromCbor<'a> for usize {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        if usize::BITS <= 32 {
            reader.read_u32().map(|v| v as usize)
        } else {
            reader.read_u64().map(|v| v as usize)
        }
    }
}

impl ToCbor for usize {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        if usize::BITS <= 32 {
            writer.write_u32(*self as u32)
        } else {
            writer.write_u64(*self as u64)
        }
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        if usize::BITS <= 32 {
            (*self as u32).size_hint()
        } else {
            (*self as u64).size_hint()
        }
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        if usize::BITS <= 32 {
            u32::max_size()
        } else {
            u64::max_size()
        }
    }
}

impl<'a> FromCbor<'a> for isize {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        if isize::BITS <= 32 {
            reader.read_i32().map(|v| v as isize)
        } else {
            reader.read_i64().map(|v| v as isize)
        }
    }
}

impl ToCbor for isize {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        if isize::BITS <= 32 {
            writer.write_i32(*self as i32)
        } else {
            writer.write_i64(*self as i64)
        }
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        if isize::BITS <= 32 {
            (*self as i32).size_hint()
        } else {
            (*self as i64).size_hint()
        }
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        if isize::BITS <= 32 {
            i32::max_size()
        } else {
            i64::max_size()
        }
    }
}

impl<'a> FromCbor<'a> for char {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        let s = reader.read_string()?;
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(Error::InvalidChar),
        }
    }
}

impl ToCbor for char {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        let mut buf = [0u8; 4];
        let s = self.encode_utf8(&mut buf);
        writer.write_string(s)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        let len = self.len_utf8();
        string_size_hint(len)
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        // A `char` is at most 4 UTF-8 bytes.
        string_size_hint(4)
    }
}

impl<'a, T> FromCbor<'a> for core::marker::PhantomData<T> {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        // The value must be consumed, or the reader would sit on the next
        // field's bytes.
        reader.read_null()?;
        Ok(core::marker::PhantomData)
    }
}

impl<T> ToCbor for core::marker::PhantomData<T> {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        // A `null`, not nothing: zero bytes would make every container holding
        // a `PhantomData` under-long, and the next value read as this one.
        writer.write_null()
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        // SAFETY: `PhantomData` always encodes as the one-byte `null`.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(1) })
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        // SAFETY: as above.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(1) })
    }
}

impl<'de, 'a> FromCbor<'de> for &'a str
where
    'de: 'a,
{
    #[inline(always)]
    fn read<R: Read<'de>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        match reader.read_string()? {
            alloc::borrow::Cow::Borrowed(s) => Ok(s),
            alloc::borrow::Cow::Owned(_) => Err(crate::Error::CannotBorrow),
        }
    }
}

impl ToCbor for str {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_string(self)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        string_size_hint(self.len())
    }
}

impl<'de, 'a> FromCbor<'de> for &'a [u8]
where
    'de: 'a,
{
    #[inline(always)]
    fn read<R: Read<'de>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        match reader.read_binary()? {
            alloc::borrow::Cow::Borrowed(s) => Ok(s),
            alloc::borrow::Cow::Owned(_) => Err(crate::Error::CannotBorrow),
        }
    }
}

impl<T: ToCbor> ToCbor for [T] {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_array_len(self.len())?;
        T::write_slice(self, writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(self.len())
    }
}

impl<'a, T: FromCbor<'a>, const N: usize> FromCbor<'a> for [T; N] {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self> {
        struct InitializedGuard<T> {
            ptr: *mut T,
            initialized: usize,
        }

        impl<T> Drop for InitializedGuard<T> {
            fn drop(&mut self) {
                // SAFETY: the first `initialized` elements were written
                // exactly once, and the backing array still exists.
                unsafe {
                    core::ptr::drop_in_place(core::ptr::slice_from_raw_parts_mut(
                        self.ptr,
                        self.initialized,
                    ));
                }
            }
        }

        reader.increment_depth()?;
        let result = (|| {
            let len = reader.check_array_len(N)?;
            let mut arr: core::mem::MaybeUninit<[T; N]> = core::mem::MaybeUninit::uninit();
            let ptr = arr.as_mut_ptr() as *mut T;
            let mut guard = InitializedGuard {
                ptr,
                initialized: 0,
            };
            for i in 0..N {
                unsafe {
                    ptr.add(i).write(T::read(reader)?);
                }
                guard.initialized += 1;
            }
            // An indefinite array ends at a break, so reading the elements is not
            // enough. This also catches an array with too many: the surplus element
            // is not the break.
            reader.finish_array(len)?;
            core::mem::forget(guard);
            Ok(unsafe { arr.assume_init() })
        })();
        reader.decrement_depth();
        result
    }
}

impl<T: ToCbor, const N: usize> ToCbor for [T; N] {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_array_len(N)?;
        T::write_slice(self, writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(N)
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(N)
    }
}

impl<'a> FromCbor<'a> for alloc::string::String {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        match reader.read_string()? {
            alloc::borrow::Cow::Borrowed(s) => Ok(s.to_string()),
            alloc::borrow::Cow::Owned(s) => Ok(s),
        }
    }
}

impl ToCbor for alloc::string::String {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_string(self)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        string_size_hint(self.len())
    }
}

impl<'de, 'a> FromCbor<'de> for alloc::borrow::Cow<'a, str>
where
    'de: 'a,
{
    #[inline(always)]
    fn read<R: Read<'de>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.read_string()
    }
}

impl ToCbor for alloc::borrow::Cow<'_, str> {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_string(self)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        string_size_hint(self.len())
    }
}

impl<'de, 'a, T> FromCbor<'de> for alloc::borrow::Cow<'a, [T]>
where
    'de: 'a,
    T: Clone + FromCbor<'de>,
{
    #[inline(always)]
    fn read<R: Read<'de>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        let mut values = alloc::vec::Vec::new();
        reader.read_array(&mut values)?;
        Ok(alloc::borrow::Cow::Owned(values))
    }
}

impl<T: Clone + ToCbor> ToCbor for alloc::borrow::Cow<'_, [T]> {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        self.as_ref().write(writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(self.len())
    }
}

impl<T: ToCbor + ?Sized> ToCbor for &T {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        T::write(self, writer)
    }

    #[inline]
    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        T::size_hint(self)
    }
}

impl<T: ToCbor + ?Sized> ToCbor for &mut T {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        T::write(self, writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        T::size_hint(self)
    }
}

impl<'a, T: FromCbor<'a>> FromCbor<'a> for Option<T> {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.read_option()
    }
}

impl<T: ToCbor> ToCbor for Option<T> {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        match self {
            Some(value) => value.write(writer),
            None => writer.write_null(),
        }
    }

    #[inline]
    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        match self {
            Some(value) => value.size_hint(),
            // SAFETY: `None` is one null byte.
            None => Some(unsafe { crate::TrustedSizeHint::new_unchecked(1) }),
        }
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        let upper = T::max_size()?.upper_bound().max(1);
        // SAFETY: either one null byte or a `T`.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(upper) })
    }
}

impl<'a, T: FromCbor<'a>, E: FromCbor<'a>> FromCbor<'a> for core::result::Result<T, E> {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.increment_depth()?;
        let result = (|| {
            let len = reader.check_array_len(2)?;
            let is_ok = reader.read_boolean()?;
            let value = if is_ok {
                core::result::Result::Ok(T::read(reader)?)
            } else {
                core::result::Result::Err(E::read(reader)?)
            };
            reader.finish_array(len)?;
            Ok(value)
        })();
        reader.decrement_depth();
        result
    }
}

impl<T: ToCbor, E: ToCbor> ToCbor for core::result::Result<T, E> {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        match self {
            Ok(value) => {
                writer.write_array_len(2)?;
                writer.write_boolean(true)?; // Ok variant
                value.write(writer)
            }
            Err(err) => {
                writer.write_array_len(2)?;
                writer.write_boolean(false)?; // Err variant
                err.write(writer)
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        let payload = match self {
            Ok(value) => value.size_hint()?,
            Err(error) => error.size_hint()?,
        };
        // A fixarray head and a boolean tag are one byte each.
        let size = payload.upper_bound().checked_add(2)?;
        // SAFETY: the wrapper and payload sizes are exact.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(size) })
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        let payload = T::max_size()?
            .upper_bound()
            .max(E::max_size()?.upper_bound());
        let upper = payload.checked_add(2)?;
        // SAFETY: the wrapper is two bytes and the payload is bounded above.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(upper) })
    }
}

impl<'a, T: FromCbor<'a>> FromCbor<'a> for alloc::vec::Vec<T> {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        let mut values = alloc::vec::Vec::new();
        reader.read_array(&mut values)?;
        Ok(values)
    }
}

impl<T: ToCbor> ToCbor for alloc::vec::Vec<T> {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        self.as_slice().write(writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(self.len())
    }
}

impl<'a, T: FromCbor<'a>> FromCbor<'a> for alloc::collections::VecDeque<T> {
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.increment_depth()?;
        let result = (|| {
            let len = reader.read_array_len()?;
            // `with_capacity` is not used: the count is off the wire, so reserving
            // it would let a header claim gigabytes before any data arrives.
            let mut vec = alloc::collections::VecDeque::new();
            let mut index = 0;
            while reader.next_element(len, &mut index)? {
                vec.push_back(T::read(reader)?);
            }
            reader.finish_array(len)?;
            Ok(vec)
        })();
        reader.decrement_depth();
        result
    }
}

impl<T: ToCbor> ToCbor for alloc::collections::VecDeque<T> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_array_len(self.len())?;
        let (front, back) = self.as_slices();
        T::write_slice(front, writer)?;
        T::write_slice(back, writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(self.len())
    }
}

impl<'a, T: FromCbor<'a>> FromCbor<'a> for alloc::collections::LinkedList<T> {
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.increment_depth()?;
        let result = (|| {
            let len = reader.read_array_len()?;
            let mut list = alloc::collections::LinkedList::new();
            let mut index = 0;
            while reader.next_element(len, &mut index)? {
                list.push_back(T::read(reader)?);
            }
            reader.finish_array(len)?;
            Ok(list)
        })();
        reader.decrement_depth();
        result
    }
}

impl<T: ToCbor> ToCbor for alloc::collections::LinkedList<T> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_array_len(self.len())?;
        for item in self {
            item.write(writer)?;
        }
        Ok(())
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(self.len())
    }
}

impl<'a, T: Ord + FromCbor<'a>> FromCbor<'a> for alloc::collections::BTreeSet<T> {
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.increment_depth()?;
        let result = (|| {
            let len = reader.read_array_len()?;
            let mut set = alloc::collections::BTreeSet::new();
            let mut index = 0;
            while reader.next_element(len, &mut index)? {
                set.insert(T::read(reader)?);
            }
            reader.finish_array(len)?;
            Ok(set)
        })();
        reader.decrement_depth();
        result
    }
}

impl<T: ToCbor> ToCbor for alloc::collections::BTreeSet<T> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_array_len(self.len())?;
        for item in self {
            item.write(writer)?;
        }
        Ok(())
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(self.len())
    }
}

impl<'a, K: Ord + FromCbor<'a>, V: FromCbor<'a>> FromCbor<'a>
    for alloc::collections::BTreeMap<K, V>
{
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.increment_depth()?;
        let result = (|| {
            let len = reader.read_map_len()?;
            let mut map = alloc::collections::BTreeMap::new();
            let mut index = 0;
            while reader.next_element(len, &mut index)? {
                let key = K::read(reader)?;
                let value = V::read(reader)?;
                // RFC 8949 Section 5.6 leaves this undefined; report rather than
                // resolve by arrival order.
                if map.insert(key, value).is_some() {
                    cold_path();
                    return Err(crate::Error::DuplicateKey);
                }
            }
            reader.finish_map(len)?;
            Ok(map)
        })();
        reader.decrement_depth();
        result
    }
}

impl<K: ToCbor, V: ToCbor> ToCbor for alloc::collections::BTreeMap<K, V> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_map_len(self.len())?;
        for (key, value) in self {
            key.write(writer)?;
            value.write(writer)?;
        }
        Ok(())
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        map_size_hint::<K, V>(self.len())
    }
}

impl<'a, T: FromCbor<'a> + Ord> FromCbor<'a> for alloc::collections::BinaryHeap<T> {
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.increment_depth()?;
        let result = (|| {
            let len = reader.read_array_len()?;

            // Not `with_capacity`: the count is off the wire.
            let mut heap = alloc::collections::BinaryHeap::new();

            let mut index = 0;
            while reader.next_element(len, &mut index)? {
                heap.push(T::read(reader)?);
            }
            reader.finish_array(len)?;
            Ok(heap)
        })();
        reader.decrement_depth();
        result
    }
}

impl<T: ToCbor + Ord> ToCbor for alloc::collections::BinaryHeap<T> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_array_len(self.len())?;
        for item in self {
            item.write(writer)?;
        }
        Ok(())
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(self.len())
    }
}

#[cfg(feature = "std")]
impl<'a, T: Hash + Eq + FromCbor<'a>> FromCbor<'a> for std::collections::HashSet<T> {
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.increment_depth()?;
        let result = (|| {
            let len = reader.read_array_len()?;

            // don't use `with_capacity` to protect against OOM attacks
            let mut set = std::collections::HashSet::new();

            let mut index = 0;
            while reader.next_element(len, &mut index)? {
                set.insert(T::read(reader)?);
            }
            reader.finish_array(len)?;
            Ok(set)
        })();
        reader.decrement_depth();
        result
    }
}

#[cfg(feature = "std")]
impl<T: ToCbor> ToCbor for std::collections::HashSet<T> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_array_len(self.len())?;
        for item in self {
            item.write(writer)?;
        }
        Ok(())
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        sequence_size_hint::<T>(self.len())
    }
}

#[cfg(feature = "std")]
impl<'a, K: Hash + Eq + FromCbor<'a>, V: FromCbor<'a>> FromCbor<'a>
    for std::collections::HashMap<K, V>
{
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.increment_depth()?;
        let result = (|| {
            let len = reader.read_map_len()?;

            // don't use `with_capacity` to protect against OOM attacks
            let mut map = std::collections::HashMap::new();

            let mut index = 0;
            while reader.next_element(len, &mut index)? {
                let key = K::read(reader)?;
                let value = V::read(reader)?;
                // RFC 8949 Section 5.6 leaves a duplicate key's meaning undefined,
                // so one is reported rather than resolved by arrival order.
                if map.insert(key, value).is_some() {
                    cold_path();
                    return Err(crate::Error::DuplicateKey);
                }
            }
            reader.finish_map(len)?;
            Ok(map)
        })();
        reader.decrement_depth();
        result
    }
}

#[cfg(feature = "std")]
impl<K: ToCbor, V: ToCbor> ToCbor for std::collections::HashMap<K, V> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_map_len(self.len())?;
        for (key, value) in self {
            key.write(writer)?;
            value.write(writer)?;
        }
        Ok(())
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        map_size_hint::<K, V>(self.len())
    }
}

impl<'a, T: FromCbor<'a>> FromCbor<'a> for alloc::boxed::Box<T> {
    #[inline(always)]
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        Ok(alloc::boxed::Box::new(T::read(reader)?))
    }
}

impl<T: ToCbor> ToCbor for alloc::boxed::Box<T> {
    #[inline(always)]
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        self.as_ref().write(writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        self.as_ref().size_hint()
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        T::max_size()
    }
}

#[cfg(feature = "std")]
impl<'a, T: FromCbor<'a>> FromCbor<'a> for std::sync::Arc<T> {
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        Ok(std::sync::Arc::new(T::read(reader)?))
    }
}

#[cfg(feature = "std")]
impl<T: ToCbor> ToCbor for std::sync::Arc<T> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        self.as_ref().write(writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        self.as_ref().size_hint()
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        T::max_size()
    }
}

impl<'a, T: FromCbor<'a>> FromCbor<'a> for alloc::rc::Rc<T> {
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        Ok(alloc::rc::Rc::new(T::read(reader)?))
    }
}

impl<T: ToCbor> ToCbor for alloc::rc::Rc<T> {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        self.as_ref().write(writer)
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        self.as_ref().size_hint()
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        T::max_size()
    }
}

impl<'a> FromCbor<'a> for () {
    fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
    where
        Self: Sized,
    {
        reader.read_null()
    }
}

impl ToCbor for () {
    fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
        writer.write_null()
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        // SAFETY: unit is one null byte.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(1) })
    }

    fn max_size() -> Option<crate::TrustedSizeHint> {
        // SAFETY: as above.
        Some(unsafe { crate::TrustedSizeHint::new_unchecked(1) })
    }
}

macro_rules! impl_tuple_cborable {
    ($len:expr; $($t:ident : $idx:tt),+ $(,)?) => {
        impl<'a, $($t: FromCbor<'a>),+> FromCbor<'a> for ($($t,)+) {
            fn read<R: Read<'a>>(reader: &mut R) -> crate::Result<Self>
            where
                Self: Sized,
            {
                reader.increment_depth()?;
                let result = (|| {
                    let len = reader.check_array_len($len)?;
                    let value = ($($t::read(reader)?,)+);
                    reader.finish_array(len)?;
                    Ok(value)
                })();
                reader.decrement_depth();
                result
            }
        }

        impl<$($t: ToCbor),+> ToCbor for ($($t,)+) {
            fn write<W: Write>(&self, writer: &mut W) -> crate::Result<()> {
                writer.write_array_len($len)?;
                $(self.$idx.write(writer)?;)+
                Ok(())
            }

            fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
                let mut size = header_size($len);
                $(size = size.checked_add(self.$idx.size_hint()?.upper_bound())?;)+
                // SAFETY: the tuple header and every element have exact-size proofs.
                Some(unsafe { crate::TrustedSizeHint::new_unchecked(size) })
            }

            fn max_size() -> Option<crate::TrustedSizeHint> {
                let mut size = header_size($len);
                $(size = size.checked_add($t::max_size()?.upper_bound())?;)+
                // SAFETY: the tuple header is exact and every field is bounded above.
                Some(unsafe { crate::TrustedSizeHint::new_unchecked(size) })
            }
        }
    };
}

impl_tuple_cborable!(2; T0:0, T1:1);
impl_tuple_cborable!(3; T0:0, T1:1, T2:2);
impl_tuple_cborable!(4; T0:0, T1:1, T2:2, T3:3);
impl_tuple_cborable!(5; T0:0, T1:1, T2:2, T3:3, T4:4);
impl_tuple_cborable!(6; T0:0, T1:1, T2:2, T3:3, T4:4, T5:5);
impl_tuple_cborable!(7; T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6);
impl_tuple_cborable!(8; T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7);
impl_tuple_cborable!(9; T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8);
impl_tuple_cborable!(10; T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9);
impl_tuple_cborable!(11; T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9, T10:10);
impl_tuple_cborable!(12; T0:0, T1:1, T2:2, T3:3, T4:4, T5:5, T6:6, T7:7, T8:8, T9:9, T10:10, T11:11);
