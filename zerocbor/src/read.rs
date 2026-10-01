use core::hint::cold_path;

#[cfg(feature = "std")]
use alloc::vec;

use crate::Error;
use crate::FromCbor;
use crate::Result;
use crate::consts::*;
use crate::write::decode_f16;

#[cold]
#[inline(never)]
fn buffer_too_small<T>() -> Result<T> {
    Err(Error::BufferTooSmall)
}

#[cold]
#[inline(never)]
fn invalid_initial_byte<T>(byte: u8) -> Result<T> {
    Err(Error::InvalidInitialByte(byte))
}

/// The maximum number of nested decoding scopes a decode accepts.
///
/// Containers and derived types consume a scope, including transparent derived
/// newtypes. Hand-written recursive decoders must balance
/// [`Read::increment_depth`] and [`Read::decrement_depth`] on success and error.
///
/// Decoding a container recurses, so an array-of-arrays-of-arrays from an
/// attacker would otherwise exhaust the stack. The value fits the 2 MiB stack an
/// unoptimized `std` test thread gets, where frames are much larger than in a
/// release build. Real CBOR is never this deep: RFC 8949 Section 4.2.2 says
/// even 4 levels is rare. Raise it only if the input needs it and the caller's
/// stack is large enough.
///
/// The bound is on reading. The writer does not count levels, so a value built
/// deeper than this encodes and is then refused by this crate's own reader. That
/// is deliberate: a counter on every write costs on every write, for a shape
/// that does not occur, and the value is the caller's own rather than an
/// attacker's.
pub const MAX_DEPTH: usize = 128;

/// Validates a text string's bytes as UTF-8, keeping a borrowed input borrowed.
///
/// Each text chunk is also validated before joining: RFC 8949 requires every
/// chunk to end at a Unicode code point boundary.
fn validate_utf8(bytes: alloc::borrow::Cow<'_, [u8]>) -> Result<alloc::borrow::Cow<'_, str>> {
    match bytes {
        alloc::borrow::Cow::Borrowed(bytes) => match core::str::from_utf8(bytes) {
            Ok(string) => Ok(alloc::borrow::Cow::Borrowed(string)),
            Err(error) => {
                cold_path();
                Err(Error::InvalidUtf8(error))
            }
        },
        alloc::borrow::Cow::Owned(bytes) => match alloc::string::String::from_utf8(bytes) {
            Ok(string) => Ok(alloc::borrow::Cow::Owned(string)),
            Err(error) => {
                cold_path();
                Err(Error::InvalidUtf8(error.utf8_error()))
            }
        },
    }
}

/// How many slots an indefinite-length array grows by when it runs out, so that
/// a `Vec` does not reallocate on every element.
const ARRAY_GROWTH: usize = 8;

/// The largest count a header may reserve up front.
///
/// A length comes off the wire, so reserving it unchecked would let a six-byte
/// header ask for gigabytes before any data arrives. Growing instead means the
/// missing data is what fails.
const MAX_PREALLOC: usize = 1024;

/// The declared length of a container, which an indefinite-length one lacks.
///
/// RFC 8949 Section 3 lets a container end with a break stop code instead of a
/// count, so a reader cannot always know how many items to expect. Both forms
/// are read through one loop: [`Len::Known`] stops after that many, an
/// [`Len::Indefinite`] at the break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Len {
    /// The container declared a count.
    Known(usize),
    /// The container ends with a break stop code instead of a count.
    Indefinite,
}

impl Len {
    /// The declared count, or `None` when the container is indefinite-length.
    #[inline(always)]
    pub const fn known(self) -> Option<usize> {
        match self {
            Len::Known(len) => Some(len),
            Len::Indefinite => None,
        }
    }

    /// Whether the container ends with a break rather than a count.
    #[inline(always)]
    pub const fn is_indefinite(self) -> bool {
        matches!(self, Len::Indefinite)
    }
}

/// A trait for reading values from a CBOR-encoded input.
/// A cursor over the elements of an array whose header has been read, so that
/// both length forms are read by one loop. The reader is passed back in per step
/// rather than borrowed, so it is never held across a call that needs it mutably.
#[derive(Debug, Clone)]
pub struct ArrayIter {
    /// The header that was read, which is what decides when the array ends.
    len: Len,
    /// How many elements have been yielded so far.
    index: usize,
}

impl ArrayIter {
    /// Advances the cursor, reporting whether another element follows.
    ///
    /// A definite array stops after its count, an indefinite one at its break.
    /// The break is not consumed: the caller closes the container with
    /// [`Read::finish_array`]. Either way the nesting depth is left balanced.
    #[inline(always)]
    pub fn next<'de, R: Read<'de> + ?Sized>(&mut self, reader: &mut R) -> Result<bool> {
        match reader.next_element(self.len, &mut self.index) {
            Ok(true) => Ok(true),
            Ok(false) => {
                reader.decrement_depth();
                Ok(false)
            }
            Err(error) => {
                reader.decrement_depth();
                Err(error)
            }
        }
    }

    /// The header this cursor was created from.
    #[inline(always)]
    pub const fn header(&self) -> Len {
        self.len
    }

    /// Whether the array's length was declared rather than terminated by a
    /// break stop code.
    #[inline(always)]
    pub const fn is_indefinite(&self) -> bool {
        self.len.is_indefinite()
    }
}

/// A cursor over the key-value pairs of a map whose header has been read. As
/// with [`ArrayIter`], the reader is passed in per step so both length forms are
/// read by one loop.
#[derive(Debug, Clone)]
pub struct MapIter {
    /// The header that was read, which is what decides when the map ends.
    len: Len,
    /// How many pairs have been yielded so far.
    index: usize,
}

impl MapIter {
    /// Advances the cursor, reporting whether another pair follows.
    ///
    /// The break is not consumed: the caller closes it with [`Read::finish_map`].
    #[inline(always)]
    pub fn next<'de, R: Read<'de> + ?Sized>(&mut self, reader: &mut R) -> Result<bool> {
        match reader.next_element(self.len, &mut self.index) {
            Ok(true) => Ok(true),
            Ok(false) => {
                reader.decrement_depth();
                Ok(false)
            }
            Err(error) => {
                reader.decrement_depth();
                Err(error)
            }
        }
    }

    /// The header this cursor was created from.
    #[inline(always)]
    pub const fn header(&self) -> Len {
        self.len
    }

    /// Whether the map's length was declared rather than terminated by a break
    /// stop code.
    #[inline(always)]
    pub const fn is_indefinite(&self) -> bool {
        self.len.is_indefinite()
    }
}

/// A trait for reading values from a CBOR-encoded input.
pub trait Read<'de> {
    /// Returns the next initial byte without consuming it.
    fn peek_initial_byte(&mut self) -> Result<u8>;

    /// Increments the current depth of nested structures.
    fn increment_depth(&mut self) -> Result<()>;

    /// Decrements the current depth of nested structures.
    fn decrement_depth(&mut self);

    /// Reads a null value from the input.
    fn read_null(&mut self) -> Result<()>;

    /// Reads a boolean value from the input.
    fn read_boolean(&mut self) -> Result<bool>;

    /// Reads an unsigned 8-bit integer from the input.
    fn read_u8(&mut self) -> Result<u8>;

    /// Reads an unsigned 16-bit integer from the input.
    fn read_u16(&mut self) -> Result<u16>;

    /// Reads an unsigned 32-bit integer from the input.
    fn read_u32(&mut self) -> Result<u32>;

    /// Reads an unsigned 64-bit integer from the input.
    fn read_u64(&mut self) -> Result<u64>;

    /// Reads a signed 8-bit integer from the input.
    fn read_i8(&mut self) -> Result<i8>;

    /// Reads a signed 16-bit integer from the input.
    fn read_i16(&mut self) -> Result<i16>;

    /// Reads a signed 32-bit integer from the input.
    fn read_i32(&mut self) -> Result<i32>;

    /// Reads a signed 64-bit integer from the input.
    fn read_i64(&mut self) -> Result<i64>;

    /// Reads a CBOR integer of either sign into the widest type the format has.
    ///
    /// A negative integer needs one bit more than a positive one of the same
    /// argument width, so both go into an `i128`: together they cover the whole
    /// `u64` range RFC 8949 Section 3.1 allows for an argument.
    fn read_integer(&mut self) -> Result<i128>;

    /// Reads a 16-bit floating-point number from the input.
    fn read_f16(&mut self) -> Result<f32>;

    /// Reads a 32-bit floating-point number from the input.
    fn read_f32(&mut self) -> Result<f32>;

    /// Reads a 64-bit floating-point number from the input.
    fn read_f64(&mut self) -> Result<f64>;

    /// Reads the array header, or [`Len::Indefinite`] if it has no count and so
    /// ends at a break stop code.
    fn read_array_len(&mut self) -> Result<Len>;

    /// Reads the map header in key-value pairs, or [`Len::Indefinite`] if it has
    /// no count and so ends at a break stop code.
    fn read_map_len(&mut self) -> Result<Len>;

    /// Reads a tag value from the input.
    fn read_tag(&mut self) -> Result<u64>;

    /// Reads the tag a type declared, or reports what was there instead.
    ///
    /// The tag is part of the value's identity, so a value read from a different
    /// one is a different value, and naming the tag found is what makes the
    /// mismatch diagnosable.
    #[inline(always)]
    fn check_tag(&mut self, expected: u64) -> Result<()> {
        let byte = self.peek_initial_byte()?;
        if byte & MAJOR_TYPE_MASK != MAJOR_TYPE_TAG {
            cold_path();
            return Err(Error::TagMismatch {
                expected,
                found: None,
            });
        }
        let found = self.read_tag()?;
        if found == expected {
            Ok(())
        } else {
            cold_path();
            Err(Error::TagMismatch {
                expected,
                found: Some(found),
            })
        }
    }

    /// Reads a simple value that is not one of the three float widths.
    ///
    /// Returns 0 through 23 for the one-byte form, or 32 through 255 for the
    /// two-byte form. Below 32 is an error: three of those numbers are the float
    /// widths and the rest are unassigned.
    fn read_simple_value(&mut self) -> Result<u8>;

    /// Reads `undefined`, a simple value distinct from `null`: `null` says a
    /// value is absent, `undefined` that it is not present for another reason.
    fn read_undefined(&mut self) -> Result<()>;

    /// Reads a UTF-8 string, borrowing from the input where possible. A chunked
    /// string is owned unless it arrived as a single chunk.
    fn read_string(&mut self) -> Result<alloc::borrow::Cow<'de, str>>;

    /// Reads a text string's raw bytes, borrowing where possible. Definite
    /// strings are not validated; indefinite text chunks are individually
    /// validated as UTF-8 to enforce their code point boundaries.
    fn read_string_bytes(&mut self) -> Result<alloc::borrow::Cow<'de, [u8]>>;

    /// Reads a byte string, borrowing where possible. A chunked one is owned
    /// unless it arrived as a single chunk.
    fn read_binary(&mut self) -> Result<alloc::borrow::Cow<'de, [u8]>>;

    /// Reports whether the next value is a map, without consuming it. A map head
    /// is never a text string head, so this tells a tagged shape from a bare one.
    #[inline(always)]
    fn at_map(&mut self) -> Result<bool> {
        Ok(self.peek_initial_byte()? & MAJOR_TYPE_MASK == MAJOR_TYPE_MAP)
    }

    /// Reports whether the next value is tagged, without consuming it. A tag head
    /// is never a map or text string head, so this tells tagged from untagged.
    #[inline(always)]
    fn at_tag(&mut self) -> Result<bool> {
        Ok(self.peek_initial_byte()? & MAJOR_TYPE_MASK == MAJOR_TYPE_TAG)
    }

    /// Consumes the break stop code that ends an indefinite-length container or
    /// string. A break is its own byte, not a null, so it is consumed here rather
    /// than through [`Read::read_null`], which would reject it.
    fn read_break(&mut self) -> Result<()>;

    /// Reads one chunk of a chunked string, or `None` at the break that ends it.
    ///
    /// A chunked string is a sequence of these, so this is how a reader walks one
    /// without concatenating first. Text chunks are individually validated as
    /// UTF-8 before they are returned.
    ///
    /// `major` is the major type of the string the chunk belongs to; a chunk of
    /// another is refused, because RFC 8949 Section 3.2.3 gives chunks their
    /// parent's type and accepting the other would read one document two ways.
    fn read_chunk(&mut self, major: u8) -> Result<Option<(Len, alloc::borrow::Cow<'de, [u8]>)>>;

    /// Reads an optional value from the input.
    /// Returns `None` if the next value is null, or `Some(value)` if it is not.
    fn read_option<T: FromCbor<'de>>(&mut self) -> Result<Option<T>>;

    /// Reads an array into an existing `Vec`, reusing its allocation.
    ///
    /// A definite array contributes its count and an indefinite one everything
    /// up to its break, so both go through one loop.
    #[inline(always)]
    fn read_array<T: FromCbor<'de>>(&mut self, out: &mut alloc::vec::Vec<T>) -> Result<()>
    where
        Self: Sized,
    {
        out.clear();
        let len = self.read_array_len()?;
        // Cap the reservation from an untrusted header. A stream cannot check
        // the count against its remaining input before reading the elements.
        if let Len::Known(count) = len
            && count <= MAX_PREALLOC
        {
            out.reserve(count);
        }
        self.increment_depth()?;
        let result = (|| -> Result<()> {
            match len {
                Len::Known(count) => {
                    for _ in 0..count {
                        out.push(T::read(self)?);
                    }
                }
                Len::Indefinite => {
                    // The header was already consumed; stop at its break.
                    let mut index = 0;
                    while self.next_element(len, &mut index)? {
                        out.push(T::read(self)?);
                    }
                }
            }
            self.finish_array(len)?;
            Ok(())
        })();
        self.decrement_depth();
        if result.is_err() {
            out.clear();
        }
        result
    }

    /// Validates that the next value is an array of `expected` elements, and
    /// consumes the header.
    ///
    /// An indefinite array is accepted, since a fixed-shape type can read the
    /// same count from it; [`Read::finish_array`] consumes the break afterwards.
    #[inline(always)]
    fn check_array_len(&mut self, expected: usize) -> Result<Len> {
        let actual = self.read_array_len()?;
        match actual {
            Len::Indefinite => Ok(actual),
            Len::Known(found) if found == expected => Ok(actual),
            Len::Known(found) => {
                cold_path();
                Err(Error::ArrayLengthMismatch {
                    expected,
                    actual: found,
                })
            }
        }
    }

    /// Validates that the next value is a map of `expected` entries, and consumes
    /// the header. As with [`Read::check_array_len`], an indefinite map is
    /// accepted and closed by [`Read::finish_map`].
    #[inline(always)]
    fn check_map_len(&mut self, expected: usize) -> Result<Len> {
        let actual = self.read_map_len()?;
        match actual {
            Len::Indefinite => Ok(actual),
            Len::Known(found) if found == expected => Ok(actual),
            Len::Known(found) => {
                cold_path();
                Err(Error::MapLengthMismatch {
                    expected,
                    actual: found,
                })
            }
        }
    }

    /// Closes an array whose header was read as `len`: a definite one checks
    /// every declared element was consumed, an indefinite one takes the break.
    ///
    /// A fixed-shape type calls this after its fields, so an array that is too
    /// long is reported rather than leaving the reader mid-value.
    #[inline(always)]
    fn finish_array(&mut self, len: Len) -> Result<()> {
        match len {
            Len::Known(_) => Ok(()),
            Len::Indefinite => self.read_break(),
        }
    }

    /// Closes a map whose header was read as `len`.
    #[inline(always)]
    fn finish_map(&mut self, len: Len) -> Result<()> {
        match len {
            Len::Known(_) => Ok(()),
            Len::Indefinite => self.read_break(),
        }
    }

    /// Reports whether another element follows. This only answers the question:
    /// closing the container is [`Read::finish_array`] or [`Read::finish_map`],
    /// so a loop has one place where the break is consumed.
    #[inline(always)]
    fn next_element(&mut self, len: Len, index: &mut usize) -> Result<bool> {
        match len {
            Len::Known(count) => {
                if *index < count {
                    *index += 1;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Len::Indefinite => {
                let at_break = self.at_break()?;
                Ok(!at_break)
            }
        }
    }

    /// Reports whether the next byte is a break stop code, without consuming it.
    #[inline(always)]
    fn at_break(&mut self) -> Result<bool> {
        Ok(self.peek_initial_byte()? == BREAK)
    }

    /// A cursor over an array's elements whose header is `len`.
    ///
    /// This is what makes both length forms readable with one loop: the caller
    /// asks for another element and the header decides whether the answer is a
    /// count or a break. `None` reads the array header first, nesting the cursor
    /// inside it.
    #[inline(always)]
    fn array_iter(&mut self) -> Result<ArrayIter>
    where
        Self: Sized,
    {
        self.array_iter_from(None)
    }

    /// A cursor over an array's elements whose header is already `len`, so a
    /// fixed-shape type does not read the header twice.
    #[inline(always)]
    fn array_iter_from(&mut self, len: Option<Len>) -> Result<ArrayIter>
    where
        Self: Sized,
    {
        let len = match len {
            Some(len) => len,
            None => self.read_array_len()?,
        };
        self.increment_depth()?;
        Ok(ArrayIter { len, index: 0 })
    }

    /// A cursor over a map's pairs whose header is `len`. As with
    /// [`Read::array_iter`], `None` reads the map header first.
    #[inline(always)]
    fn map_iter(&mut self) -> Result<MapIter>
    where
        Self: Sized,
    {
        self.map_iter_from(None)
    }

    /// A cursor over a map's pairs whose header is already `len`.
    #[inline(always)]
    fn map_iter_from(&mut self, len: Option<Len>) -> Result<MapIter>
    where
        Self: Sized,
    {
        let len = match len {
            Some(len) => len,
            None => self.read_map_len()?,
        };
        self.increment_depth()?;
        Ok(MapIter { len, index: 0 })
    }

    /// Reads a CBOR integer of either sign.
    ///
    /// CBOR splits them across major type 0 and 1, so a signed reader must take
    /// both; rejecting major type 0 would break every non-negative value.
    fn read_signed(&mut self) -> Result<i64>;

    /// Consumes exactly one CBOR value from the input, regardless of its type.
    /// Used to skip over unknown keys' values without needing to know their type.
    fn skip_value(&mut self) -> Result<()>;
}

pub(crate) struct SliceReader<'de> {
    data: &'de [u8],
    pos: usize,
    depth: usize,
}

impl<'de> SliceReader<'de> {
    pub fn new(data: &'de [u8]) -> Self {
        Self {
            data,
            pos: 0,
            depth: 0,
        }
    }

    #[inline(always)]
    fn peek_byte(&mut self) -> Result<u8> {
        if self.pos < self.data.len() {
            unsafe { Ok(*self.data.get_unchecked(self.pos)) }
        } else {
            cold_path();
            buffer_too_small()
        }
    }

    #[inline(always)]
    fn peek_slice(&mut self, len: usize) -> Result<&'de [u8]> {
        if len <= self.data.len() - self.pos {
            unsafe { Ok(self.data.get_unchecked(self.pos..(self.pos + len))) }
        } else {
            cold_path();
            buffer_too_small()
        }
    }

    #[inline(always)]
    fn take_byte(&mut self) -> Result<u8> {
        if self.pos < self.data.len() {
            let byte = unsafe { *self.data.get_unchecked(self.pos) };
            self.pos += 1;
            Ok(byte)
        } else {
            cold_path();
            buffer_too_small()
        }
    }

    #[inline(always)]
    fn take_slice(&mut self, len: usize) -> Result<&'de [u8]> {
        let slice = self.peek_slice(len)?;
        self.pos += len;
        Ok(slice)
    }

    #[inline(always)]
    fn take_array_from(&mut self, len: usize) -> Option<&'de [u8]> {
        let remaining = self.data.len().checked_sub(self.pos)?;
        if len <= remaining {
            let slice = &self.data[self.pos..self.pos + len];
            self.pos += len;
            Some(slice)
        } else {
            None
        }
    }

    #[inline(always)]
    fn take_array<const N: usize>(&mut self) -> Result<&'de [u8; N]> {
        if N <= self.data.len() - self.pos {
            let array = unsafe { &*(self.data.as_ptr().add(self.pos) as *const [u8; N]) };
            self.pos += N;
            Ok(array)
        } else {
            cold_path();
            buffer_too_small()
        }
    }

    #[inline(always)]
    fn read_additional_info(&mut self, byte: u8) -> Result<u64> {
        let info = byte & !MAJOR_TYPE_MASK;
        match info {
            0..=23 => Ok(info as u64),
            ADDITIONAL_INFO_1_BYTE => {
                let val = self.take_byte()? as u64;
                Ok(val)
            }
            ADDITIONAL_INFO_2_BYTES => {
                let bytes = self.take_array::<2>()?;
                Ok(u16::from_be_bytes(*bytes) as u64)
            }
            ADDITIONAL_INFO_4_BYTES => {
                let bytes = self.take_array::<4>()?;
                Ok(u32::from_be_bytes(*bytes) as u64)
            }
            ADDITIONAL_INFO_8_BYTES => {
                let bytes = self.take_array::<8>()?;
                Ok(u64::from_be_bytes(*bytes))
            }
            ADDITIONAL_INFO_INDEFINITE => {
                cold_path();
                Err(Error::InvalidAdditionalInfo(ADDITIONAL_INFO_INDEFINITE))
            }
            _ => {
                cold_path();
                Err(Error::InvalidInitialByte(byte))
            }
        }
    }

    #[inline(always)]
    fn skip_array_values(&mut self, len: usize) -> Result<()> {
        self.increment_depth()?;
        let result = (0..len).try_for_each(|_| self.skip_value());
        self.decrement_depth();
        result
    }

    #[inline(always)]
    fn skip_map_entries(&mut self, len: usize) -> Result<()> {
        self.increment_depth()?;
        let result = (0..len).try_for_each(|_| {
            self.skip_value()?;
            self.skip_value()
        });
        self.decrement_depth();
        result
    }

    /// Skips an indefinite-length array, up to its break stop code.
    #[inline(always)]
    fn skip_array_until_break(&mut self) -> Result<()> {
        self.increment_depth()?;
        let result = (|| -> Result<()> {
            while !self.at_break()? {
                self.skip_value()?;
            }
            self.read_break()
        })();
        self.decrement_depth();
        result
    }

    /// Skips an indefinite-length map, up to its break stop code.
    #[inline(always)]
    fn skip_map_until_break(&mut self) -> Result<()> {
        self.increment_depth()?;
        let result = (|| -> Result<()> {
            while !self.at_break()? {
                self.skip_value()?;
                self.skip_value()?;
            }
            self.read_break()
        })();
        self.decrement_depth();
        result
    }

    /// Skips a chunked string's chunks, up to its break stop code.
    #[inline(always)]
    fn skip_chunks(&mut self, major: u8) -> Result<()> {
        while self.read_chunk(major)?.is_some() {}
        Ok(())
    }
    /// Reads a text string whose head byte has been consumed.
    ///
    /// A count is one contiguous run, so it is borrowed. No count means the
    /// bytes are not contiguous, so they are concatenated.
    #[inline(always)]
    fn read_string_head(&mut self, byte: u8) -> Result<alloc::borrow::Cow<'de, [u8]>> {
        match self.read_additional_info(byte) {
            Ok(len) => Ok(alloc::borrow::Cow::Borrowed(self.take_slice(
                usize::try_from(len).map_err(|_| Error::IntegerOutOfRange)?,
            )?)),
            Err(Error::InvalidAdditionalInfo(_)) => {
                self.read_chunked(crate::consts::MAJOR_TYPE_TEXT_STRING)
            }
            Err(error) => Err(error),
        }
    }

    /// Reads a byte string whose head byte has been consumed.
    #[inline(always)]
    fn read_binary_head(&mut self, byte: u8) -> Result<alloc::borrow::Cow<'de, [u8]>> {
        match self.read_additional_info(byte) {
            Ok(len) => Ok(alloc::borrow::Cow::Borrowed(self.take_slice(
                usize::try_from(len).map_err(|_| Error::IntegerOutOfRange)?,
            )?)),
            Err(Error::InvalidAdditionalInfo(_)) => {
                self.read_chunked(crate::consts::MAJOR_TYPE_BYTE_STRING)
            }
            Err(error) => Err(error),
        }
    }

    /// Reads a chunked string's chunks, up to its break.
    ///
    /// A single chunk is borrowed, the case a producer is most likely to emit;
    /// two or more are copied into a buffer that outlives the input.
    #[cold]
    #[inline(never)]
    fn read_chunked(&mut self, major: u8) -> Result<alloc::borrow::Cow<'de, [u8]>> {
        let first = match self.read_chunk(major)? {
            Some((_, bytes)) => bytes,
            // `(_ )` is the empty string.
            None => return Ok(alloc::borrow::Cow::Owned(alloc::vec::Vec::new())),
        };
        if self.at_break()? {
            self.read_break()?;
            return Ok(first);
        }
        let mut out = match first {
            alloc::borrow::Cow::Borrowed(bytes) => bytes.to_vec(),
            alloc::borrow::Cow::Owned(bytes) => bytes,
        };
        while let Some((_, chunk)) = self.read_chunk(major)? {
            out.extend_from_slice(&chunk);
        }
        Ok(alloc::borrow::Cow::Owned(out))
    }
}

impl<'de> Read<'de> for SliceReader<'de> {
    #[inline(always)]
    fn peek_initial_byte(&mut self) -> Result<u8> {
        self.peek_byte()
    }

    #[inline(always)]
    fn increment_depth(&mut self) -> Result<()> {
        if self.depth >= MAX_DEPTH {
            cold_path();
            Err(Error::DepthLimitExceeded)
        } else {
            self.depth += 1;
            Ok(())
        }
    }

    #[inline(always)]
    fn decrement_depth(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        } else {
            cold_path();
        }
    }

    #[inline(always)]
    fn read_null(&mut self) -> Result<()> {
        let byte = self.take_byte()?;
        if byte == SIMPLE_VALUE_NULL {
            Ok(())
        } else {
            cold_path();
            invalid_initial_byte(byte)
        }
    }

    #[inline(always)]
    fn read_boolean(&mut self) -> Result<bool> {
        let byte = self.take_byte()?;
        match byte {
            SIMPLE_VALUE_TRUE => Ok(true),
            SIMPLE_VALUE_FALSE => Ok(false),
            _ => {
                cold_path();
                invalid_initial_byte(byte)
            }
        }
    }

    #[inline(always)]
    fn read_simple_value(&mut self) -> Result<u8> {
        let byte = self.take_byte()?;
        match byte & !MAJOR_TYPE_MASK {
            0..=23 => Ok(byte & !MAJOR_TYPE_MASK),
            // Additional information 24 carries the value in the next byte,
            // which is how simple values 32 and up are written. RFC 8949
            // Section 3.3 gives that form no other meaning, so below 32 is
            // refused: the writer only writes it for 32 and up, and a value the
            // reader accepts must be one the writer can produce.
            ADDITIONAL_INFO_1_BYTE => {
                let value = self.take_byte()?;
                match value {
                    0..=31 => {
                        cold_path();
                        Err(Error::InvalidSimpleValue(value))
                    }
                    _ => Ok(value),
                }
            }
            // 25 through 27 are the float widths and 31 is the break, so
            // neither is a simple value; 28 through 30 are unassigned.
            _ => {
                cold_path();
                Err(Error::InvalidSimpleValue(byte & !MAJOR_TYPE_MASK))
            }
        }
    }

    #[inline(always)]
    fn read_undefined(&mut self) -> Result<()> {
        let byte = self.take_byte()?;
        if byte == SIMPLE_VALUE_UNDEFINED {
            Ok(())
        } else {
            cold_path();
            invalid_initial_byte(byte)
        }
    }

    #[inline(always)]
    fn read_u8(&mut self) -> Result<u8> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_UNSIGNED_INT {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let value = self.read_additional_info(byte)?;
        if value > u8::MAX as u64 {
            cold_path();
            return Err(Error::InvalidInitialByte(byte));
        }
        Ok(value as u8)
    }

    #[inline(always)]
    fn read_u16(&mut self) -> Result<u16> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_UNSIGNED_INT {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let value = self.read_additional_info(byte)?;
        if value > u16::MAX as u64 {
            cold_path();
            return Err(Error::InvalidInitialByte(byte));
        }
        Ok(value as u16)
    }

    #[inline(always)]
    fn read_u32(&mut self) -> Result<u32> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_UNSIGNED_INT {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let value = self.read_additional_info(byte)?;
        if value > u32::MAX as u64 {
            cold_path();
            return Err(Error::InvalidInitialByte(byte));
        }
        Ok(value as u32)
    }

    #[inline(always)]
    fn read_u64(&mut self) -> Result<u64> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_UNSIGNED_INT {
            cold_path();
            return invalid_initial_byte(byte);
        }
        self.read_additional_info(byte)
    }

    #[inline(always)]
    fn read_i8(&mut self) -> Result<i8> {
        self.read_signed()
            .and_then(|v| i8::try_from(v).map_err(|_| Error::IntegerOutOfRange))
    }

    #[inline(always)]
    fn read_i16(&mut self) -> Result<i16> {
        self.read_signed()
            .and_then(|v| i16::try_from(v).map_err(|_| Error::IntegerOutOfRange))
    }

    #[inline(always)]
    fn read_i32(&mut self) -> Result<i32> {
        self.read_signed()
            .and_then(|v| i32::try_from(v).map_err(|_| Error::IntegerOutOfRange))
    }

    fn read_integer(&mut self) -> Result<i128> {
        let byte = self.take_byte()?;
        let value = self.read_additional_info(byte)?;
        match byte & MAJOR_TYPE_MASK {
            // Both fit: an argument is at most 64 bits, and the negative form
            // is one below its negation, so `-2^64` at the bottom.
            MAJOR_TYPE_UNSIGNED_INT => Ok(value as i128),
            MAJOR_TYPE_NEGATIVE_INT => Ok(-1i128 - value as i128),
            _ => {
                cold_path();
                invalid_initial_byte(byte)
            }
        }
    }

    #[inline(always)]
    fn read_i64(&mut self) -> Result<i64> {
        self.read_signed()
    }

    /// Reads an integer of either sign, accepting major type 0 and 1.
    #[inline(always)]
    fn read_signed(&mut self) -> Result<i64> {
        let byte = self.take_byte()?;
        let value = self.read_additional_info(byte)?;
        match byte & MAJOR_TYPE_MASK {
            MAJOR_TYPE_UNSIGNED_INT => {
                // A `u64` can exceed `i64::MAX`, so the cast is checked.
                i64::try_from(value).map_err(|_| Error::IntegerOutOfRange)
            }
            MAJOR_TYPE_NEGATIVE_INT => {
                // `-1 - value` reaches `-2^64` when an 8-byte argument has its
                // top bit set, so the subtraction is wider and the result
                // range-checked. Casting to `i64` first would make the argument
                // negative and overflow, returning a positive number.
                i64::try_from(-1i128 - value as i128).map_err(|_| Error::IntegerOutOfRange)
            }
            _ => {
                cold_path();
                invalid_initial_byte(byte)
            }
        }
    }

    #[inline(always)]
    fn read_f16(&mut self) -> Result<f32> {
        let byte = self.take_byte()?;
        if byte != FLOAT16_MARKER {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let bytes = self.take_array::<2>()?;
        let bits = u16::from_be_bytes(*bytes);
        Ok(decode_f16(bits))
    }

    #[inline(always)]
    fn read_f32(&mut self) -> Result<f32> {
        let byte = self.take_byte()?;
        if byte != FLOAT32_MARKER {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let bytes = self.take_array::<4>()?;
        Ok(f32::from_be_bytes(*bytes))
    }

    #[inline(always)]
    fn read_f64(&mut self) -> Result<f64> {
        let byte = self.take_byte()?;
        if byte != FLOAT64_MARKER {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let bytes = self.take_array::<8>()?;
        Ok(f64::from_be_bytes(*bytes))
    }

    #[inline(always)]
    fn read_chunk(&mut self, major: u8) -> Result<Option<(Len, alloc::borrow::Cow<'de, [u8]>)>> {
        if self.at_break()? {
            self.read_break()?;
            return Ok(None);
        }
        let byte = self.take_byte()?;
        // RFC 8949 Section 3.2.3 gives chunks their parent's type, so the other
        // major type would let one document be read two ways.
        if byte & MAJOR_TYPE_MASK != major {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let len = usize::try_from(self.read_additional_info(byte)?)
            .map_err(|_| Error::IntegerOutOfRange)?;
        let bytes = match self.take_array_from(len) {
            Some(array) => array,
            None => return buffer_too_small(),
        };
        if major == MAJOR_TYPE_TEXT_STRING {
            core::str::from_utf8(bytes).map_err(Error::InvalidUtf8)?;
        }
        Ok(Some((Len::Known(len), alloc::borrow::Cow::Borrowed(bytes))))
    }

    #[inline(always)]
    fn read_break(&mut self) -> Result<()> {
        let byte = self.take_byte()?;
        if byte == BREAK {
            Ok(())
        } else {
            cold_path();
            invalid_initial_byte(byte)
        }
    }

    #[inline(always)]
    fn read_array_len(&mut self) -> Result<Len> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_ARRAY {
            cold_path();
            return invalid_initial_byte(byte);
        }
        // 31 means no count: the container ends at a break, which the caller's
        // loop stops on.
        Ok(match self.read_additional_info(byte) {
            Ok(len) => Len::Known(usize::try_from(len).map_err(|_| Error::IntegerOutOfRange)?),
            Err(Error::InvalidAdditionalInfo(_)) => Len::Indefinite,
            Err(error) => return Err(error),
        })
    }

    #[inline(always)]
    fn read_map_len(&mut self) -> Result<Len> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_MAP {
            cold_path();
            return invalid_initial_byte(byte);
        }
        // 31 means no count: the container ends at a break, which the caller's
        // loop stops on.
        Ok(match self.read_additional_info(byte) {
            Ok(len) => Len::Known(usize::try_from(len).map_err(|_| Error::IntegerOutOfRange)?),
            Err(Error::InvalidAdditionalInfo(_)) => Len::Indefinite,
            Err(error) => return Err(error),
        })
    }

    #[inline(always)]
    fn read_tag(&mut self) -> Result<u64> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_TAG {
            cold_path();
            return invalid_initial_byte(byte);
        }
        self.read_additional_info(byte)
    }

    #[inline(always)]
    fn read_string(&mut self) -> Result<alloc::borrow::Cow<'de, str>> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_TEXT_STRING {
            cold_path();
            return invalid_initial_byte(byte);
        }
        // Each chunk is valid UTF-8; the joined string is also
        // validated as a whole.
        validate_utf8(self.read_string_head(byte)?)
    }

    #[inline(always)]
    fn read_string_bytes(&mut self) -> Result<alloc::borrow::Cow<'de, [u8]>> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_TEXT_STRING {
            cold_path();
            return invalid_initial_byte(byte);
        }
        self.read_string_head(byte)
    }

    #[inline(always)]
    fn read_binary(&mut self) -> Result<alloc::borrow::Cow<'de, [u8]>> {
        let byte = self.take_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_BYTE_STRING {
            cold_path();
            return invalid_initial_byte(byte);
        }
        self.read_binary_head(byte)
    }

    #[inline(always)]
    fn read_option<T: FromCbor<'de>>(&mut self) -> Result<Option<T>> {
        let byte = self.peek_byte()?;
        if byte == SIMPLE_VALUE_NULL {
            self.pos += 1;
            Ok(None)
        } else {
            Ok(Some(T::read(self)?))
        }
    }

    #[inline(always)]
    fn read_array<T: FromCbor<'de>>(&mut self, out: &mut alloc::vec::Vec<T>) -> Result<()> {
        out.clear();
        let len = self.read_array_len()?;

        // Every value is at least one byte, so a count past the input is
        // impossible. Rejecting it before it grows anything is what keeps a
        // six-byte header from asking for gigabytes.
        if let Len::Known(count) = len
            && self.data.len() - self.pos < count
        {
            cold_path();
            return Err(Error::BufferTooSmall);
        }

        if let Len::Known(count) = len
            && out.capacity() < count
        {
            out.reserve(count);
        }
        self.increment_depth()?;
        let result = (|| -> Result<()> {
            let mut index = 0;
            let mut initialized = 0usize;
            while self.next_element(len, &mut index)? {
                let value = T::read(self)?;
                if initialized == out.capacity() {
                    out.reserve(initialized.max(ARRAY_GROWTH));
                }
                // SAFETY: the reserve guarantees a writable slot. Advancing
                // the length immediately lets errors drop initialized elements.
                unsafe {
                    out.as_mut_ptr().add(initialized).write(value);
                    out.set_len(initialized + 1);
                }
                initialized += 1;
            }
            self.finish_array(len)
        })();
        self.decrement_depth();
        if result.is_err() {
            out.clear();
        }
        result
    }

    fn skip_value(&mut self) -> Result<()> {
        let byte = self.peek_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        let info = byte & !MAJOR_TYPE_MASK;

        match major_type {
            MAJOR_TYPE_UNSIGNED_INT | MAJOR_TYPE_NEGATIVE_INT | MAJOR_TYPE_TAG => {
                self.pos += 1;
                self.skip_additional_info(info)?;
            }
            MAJOR_TYPE_BYTE_STRING | MAJOR_TYPE_TEXT_STRING => {
                self.pos += 1;
                if info == ADDITIONAL_INFO_INDEFINITE {
                    self.skip_chunks(major_type)?;
                } else {
                    let len = usize::try_from(self.skip_additional_info(info)?)
                        .map_err(|_| Error::IntegerOutOfRange)?;
                    self.take_slice(len)?;
                }
            }
            MAJOR_TYPE_ARRAY => {
                self.pos += 1;
                if info == ADDITIONAL_INFO_INDEFINITE {
                    self.skip_array_until_break()?;
                } else {
                    let len = usize::try_from(self.skip_additional_info(info)?)
                        .map_err(|_| Error::IntegerOutOfRange)?;
                    self.skip_array_values(len)?;
                }
            }
            MAJOR_TYPE_MAP => {
                self.pos += 1;
                if info == ADDITIONAL_INFO_INDEFINITE {
                    self.skip_map_until_break()?;
                } else {
                    let len = usize::try_from(self.skip_additional_info(info)?)
                        .map_err(|_| Error::IntegerOutOfRange)?;
                    self.skip_map_entries(len)?;
                }
            }
            MAJOR_TYPE_SIMPLE_FLOAT => match info {
                0..=23 | 31 => {
                    self.pos += 1;
                }
                ADDITIONAL_INFO_1_BYTE => {
                    self.take_slice(2)?;
                }
                ADDITIONAL_INFO_2_BYTES => {
                    self.take_slice(3)?;
                }
                ADDITIONAL_INFO_4_BYTES => {
                    self.take_slice(5)?;
                }
                ADDITIONAL_INFO_8_BYTES => {
                    self.take_slice(9)?;
                }
                _ => {
                    cold_path();
                    return Err(Error::InvalidInitialByte(byte));
                }
            },
            _ => {
                cold_path();
                return Err(Error::InvalidInitialByte(byte));
            }
        }
        Ok(())
    }
}

impl<'de> SliceReader<'de> {
    #[inline(always)]
    fn skip_additional_info(&mut self, info: u8) -> Result<u64> {
        match info {
            0..=23 => Ok(info as u64),
            ADDITIONAL_INFO_1_BYTE => {
                let val = self.take_byte()? as u64;
                Ok(val)
            }
            ADDITIONAL_INFO_2_BYTES => {
                let bytes = self.take_array::<2>()?;
                Ok(u16::from_be_bytes(*bytes) as u64)
            }
            ADDITIONAL_INFO_4_BYTES => {
                let bytes = self.take_array::<4>()?;
                Ok(u32::from_be_bytes(*bytes) as u64)
            }
            ADDITIONAL_INFO_8_BYTES => {
                let bytes = self.take_array::<8>()?;
                Ok(u64::from_be_bytes(*bytes))
            }
            ADDITIONAL_INFO_INDEFINITE => {
                cold_path();
                Err(Error::InvalidAdditionalInfo(ADDITIONAL_INFO_INDEFINITE))
            }
            _ => {
                cold_path();
                Err(Error::InvalidInitialByte(info))
            }
        }
    }
}

#[cfg(feature = "std")]
pub(crate) struct IOReader<R: std::io::Read> {
    reader: R,
    depth: usize,
    peeked: Option<u8>,
}

#[cfg(feature = "std")]
impl<R: std::io::Read> IOReader<R> {
    /// Reads a text string whose head byte has already been consumed.
    #[inline(always)]
    fn read_string_head(&mut self, byte: u8) -> Result<alloc::vec::Vec<u8>> {
        match self.read_additional_info(byte) {
            Ok(len) => {
                self.read_exact_vec(usize::try_from(len).map_err(|_| Error::IntegerOutOfRange)?)
            }
            Err(Error::InvalidAdditionalInfo(_)) => {
                self.read_chunked(crate::consts::MAJOR_TYPE_TEXT_STRING)
            }
            Err(error) => Err(error),
        }
    }

    /// Reads a byte string whose head byte has already been consumed.
    #[inline(always)]
    fn read_binary_head(&mut self, byte: u8) -> Result<alloc::vec::Vec<u8>> {
        match self.read_additional_info(byte) {
            Ok(len) => {
                self.read_exact_vec(usize::try_from(len).map_err(|_| Error::IntegerOutOfRange)?)
            }
            Err(Error::InvalidAdditionalInfo(_)) => {
                self.read_chunked(crate::consts::MAJOR_TYPE_BYTE_STRING)
            }
            Err(error) => Err(error),
        }
    }

    /// Reads the chunks of a chunked string, up to its break stop code.
    ///
    /// Every text chunk is validated before joining, so a Unicode code point
    /// cannot straddle chunk boundaries.
    #[cold]
    #[inline(never)]
    fn read_chunked(&mut self, major: u8) -> Result<alloc::vec::Vec<u8>> {
        let mut out = alloc::vec::Vec::new();
        while let Some((_, chunk)) = self.read_chunk(major)? {
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    pub fn new(reader: R) -> Self {
        Self {
            reader,
            depth: 0,
            peeked: None,
        }
    }

    #[inline(always)]
    fn read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.reader.read_exact(buf).map_err(Error::IoError)
    }

    #[inline(always)]
    fn read_byte(&mut self) -> Result<u8> {
        if let Some(byte) = self.peeked.take() {
            Ok(byte)
        } else {
            let mut buf = [0u8; 1];
            self.read_exact(&mut buf)?;
            Ok(buf[0])
        }
    }

    #[inline(always)]
    fn read_exact_vec(&mut self, len: usize) -> Result<alloc::vec::Vec<u8>> {
        const CHUNK_SIZE: usize = 8192;

        if len == 0 {
            return Ok(alloc::vec::Vec::new());
        } else if len < CHUNK_SIZE {
            let mut buf = vec![0u8; len];
            self.reader.read_exact(&mut buf).map_err(Error::IoError)?;
            return Ok(buf);
        }

        // Keep the 8 KiB scratch buffer in a separate frame. In debug builds,
        // inlining it here would retain that buffer at every recursive level,
        // even when this branch is not taken for a short map key.
        let mut out = alloc::vec::Vec::new();
        self.read_exact_into(len, &mut out)?;

        Ok(out)
    }

    #[inline(always)]
    fn read_additional_info(&mut self, byte: u8) -> Result<u64> {
        let info = byte & !MAJOR_TYPE_MASK;
        match info {
            0..=23 => Ok(info as u64),
            ADDITIONAL_INFO_1_BYTE => {
                let val = self.read_byte()? as u64;
                Ok(val)
            }
            ADDITIONAL_INFO_2_BYTES => {
                let mut bytes = [0; 2];
                self.read_exact(&mut bytes)?;
                Ok(u16::from_be_bytes(bytes) as u64)
            }
            ADDITIONAL_INFO_4_BYTES => {
                let mut bytes = [0; 4];
                self.read_exact(&mut bytes)?;
                Ok(u32::from_be_bytes(bytes) as u64)
            }
            ADDITIONAL_INFO_8_BYTES => {
                let mut bytes = [0; 8];
                self.read_exact(&mut bytes)?;
                Ok(u64::from_be_bytes(bytes))
            }
            ADDITIONAL_INFO_INDEFINITE => {
                cold_path();
                Err(Error::InvalidAdditionalInfo(ADDITIONAL_INFO_INDEFINITE))
            }
            _ => {
                cold_path();
                Err(Error::InvalidInitialByte(byte))
            }
        }
    }
}

#[cfg(feature = "std")]
impl<'de, R: std::io::Read> Read<'de> for IOReader<R> {
    #[inline(always)]
    fn peek_initial_byte(&mut self) -> Result<u8> {
        if let Some(byte) = self.peeked {
            Ok(byte)
        } else {
            let byte = self.read_byte()?;
            self.peeked = Some(byte);
            Ok(byte)
        }
    }

    #[inline(always)]
    fn increment_depth(&mut self) -> Result<()> {
        if self.depth >= MAX_DEPTH {
            cold_path();
            Err(Error::DepthLimitExceeded)
        } else {
            self.depth += 1;
            Ok(())
        }
    }

    #[inline(always)]
    fn decrement_depth(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
        } else {
            cold_path();
        }
    }

    #[inline(always)]
    fn read_null(&mut self) -> Result<()> {
        let byte = self.read_byte()?;
        if byte == SIMPLE_VALUE_NULL {
            Ok(())
        } else {
            cold_path();
            invalid_initial_byte(byte)
        }
    }

    #[inline(always)]
    fn read_boolean(&mut self) -> Result<bool> {
        let byte = self.read_byte()?;
        match byte {
            SIMPLE_VALUE_TRUE => Ok(true),
            SIMPLE_VALUE_FALSE => Ok(false),
            _ => {
                cold_path();
                invalid_initial_byte(byte)
            }
        }
    }

    #[inline(always)]
    fn read_simple_value(&mut self) -> Result<u8> {
        let byte = self.read_byte()?;
        match byte & !MAJOR_TYPE_MASK {
            0..=23 => Ok(byte & !MAJOR_TYPE_MASK),
            // Additional information 24 carries the value in the next byte,
            // which is how simple values 32 and up are written. RFC 8949
            // Section 3.3 gives that two-byte form no other meaning, so a value
            // below 32 here is refused: the writer only ever writes this form for
            // 32 and up, and a value the reader accepts has to be one the writer
            // can produce. 24 through 31 are the float widths, the break and the
            // unassigned numbers, and 20 through 23 are `false`, `true`, `null`
            // and `undefined`, which have their own single-byte forms and their
            // own `Value` variants.
            ADDITIONAL_INFO_1_BYTE => {
                let value = self.read_byte()?;
                match value {
                    0..=31 => {
                        cold_path();
                        Err(Error::InvalidSimpleValue(value))
                    }
                    _ => Ok(value),
                }
            }
            // 25 through 27 are the float widths and 31 is the break, so
            // neither is a simple value; 28 through 30 are unassigned.
            _ => {
                cold_path();
                Err(Error::InvalidSimpleValue(byte & !MAJOR_TYPE_MASK))
            }
        }
    }

    #[inline(always)]
    fn read_undefined(&mut self) -> Result<()> {
        let byte = self.read_byte()?;
        if byte == SIMPLE_VALUE_UNDEFINED {
            Ok(())
        } else {
            cold_path();
            invalid_initial_byte(byte)
        }
    }

    #[inline(always)]
    fn read_u8(&mut self) -> Result<u8> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_UNSIGNED_INT {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let value = self.read_additional_info(byte)?;
        if value > u8::MAX as u64 {
            cold_path();
            return Err(Error::InvalidInitialByte(byte));
        }
        Ok(value as u8)
    }

    #[inline(always)]
    fn read_u16(&mut self) -> Result<u16> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_UNSIGNED_INT {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let value = self.read_additional_info(byte)?;
        if value > u16::MAX as u64 {
            cold_path();
            return Err(Error::InvalidInitialByte(byte));
        }
        Ok(value as u16)
    }

    #[inline(always)]
    fn read_u32(&mut self) -> Result<u32> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_UNSIGNED_INT {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let value = self.read_additional_info(byte)?;
        if value > u32::MAX as u64 {
            cold_path();
            return Err(Error::InvalidInitialByte(byte));
        }
        Ok(value as u32)
    }

    #[inline(always)]
    fn read_u64(&mut self) -> Result<u64> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_UNSIGNED_INT {
            cold_path();
            return invalid_initial_byte(byte);
        }
        self.read_additional_info(byte)
    }

    #[inline(always)]
    fn read_i8(&mut self) -> Result<i8> {
        self.read_signed()
            .and_then(|v| i8::try_from(v).map_err(|_| Error::IntegerOutOfRange))
    }

    #[inline(always)]
    fn read_i16(&mut self) -> Result<i16> {
        self.read_signed()
            .and_then(|v| i16::try_from(v).map_err(|_| Error::IntegerOutOfRange))
    }

    #[inline(always)]
    fn read_i32(&mut self) -> Result<i32> {
        self.read_signed()
            .and_then(|v| i32::try_from(v).map_err(|_| Error::IntegerOutOfRange))
    }

    fn read_integer(&mut self) -> Result<i128> {
        let byte = self.read_byte()?;
        let value = self.read_additional_info(byte)?;
        match byte & MAJOR_TYPE_MASK {
            // Both fit an `i128`: an argument is at most 64 bits, and the
            // negative form is one below its negation, so `-2^64` at the bottom.
            MAJOR_TYPE_UNSIGNED_INT => Ok(value as i128),
            MAJOR_TYPE_NEGATIVE_INT => Ok(-1i128 - value as i128),
            _ => {
                cold_path();
                invalid_initial_byte(byte)
            }
        }
    }

    #[inline(always)]
    fn read_i64(&mut self) -> Result<i64> {
        self.read_signed()
    }

    /// Reads an integer of either sign, accepting major type 0 and 1.
    #[inline(always)]
    fn read_signed(&mut self) -> Result<i64> {
        let byte = self.read_byte()?;
        let value = self.read_additional_info(byte)?;
        match byte & MAJOR_TYPE_MASK {
            MAJOR_TYPE_UNSIGNED_INT => {
                // A `u64` can exceed `i64::MAX`, so the cast is checked.
                i64::try_from(value).map_err(|_| Error::IntegerOutOfRange)
            }
            MAJOR_TYPE_NEGATIVE_INT => {
                // `-1 - value` can reach `-2^64` when an 8-byte argument has its
                // top bit set, so the subtraction is done in the wider type and
                // the result is range-checked. Casting the argument to an `i64`
                // first would make it negative, and the subtraction would then
                // overflow and hand back a *positive* number for a negative one.
                i64::try_from(-1i128 - value as i128).map_err(|_| Error::IntegerOutOfRange)
            }
            _ => {
                cold_path();
                invalid_initial_byte(byte)
            }
        }
    }

    #[inline(always)]
    fn read_f16(&mut self) -> Result<f32> {
        let byte = self.read_byte()?;
        if byte != FLOAT16_MARKER {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let mut bytes = [0; 2];
        self.read_exact(&mut bytes)?;
        let bits = u16::from_be_bytes(bytes);
        Ok(decode_f16(bits))
    }

    #[inline(always)]
    fn read_f32(&mut self) -> Result<f32> {
        let byte = self.read_byte()?;
        if byte != FLOAT32_MARKER {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let mut bytes = [0; 4];
        self.read_exact(&mut bytes)?;
        Ok(f32::from_be_bytes(bytes))
    }

    #[inline(always)]
    fn read_f64(&mut self) -> Result<f64> {
        let byte = self.read_byte()?;
        if byte != FLOAT64_MARKER {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let mut bytes = [0; 8];
        self.read_exact(&mut bytes)?;
        Ok(f64::from_be_bytes(bytes))
    }

    #[inline(always)]
    fn read_chunk(&mut self, major: u8) -> Result<Option<(Len, alloc::borrow::Cow<'de, [u8]>)>> {
        if self.at_break()? {
            self.read_break()?;
            return Ok(None);
        }
        let byte = self.read_byte()?;
        // RFC 8949 Section 3.2.3 says the chunks of an indefinite-length text
        // string are text strings and those of an indefinite-length byte string
        // are byte strings. A chunk of the other kind is a different major type,
        // so accepting it would let one document be read two ways.
        if byte & MAJOR_TYPE_MASK != major {
            cold_path();
            return invalid_initial_byte(byte);
        }
        let len = usize::try_from(self.read_additional_info(byte)?)
            .map_err(|_| Error::IntegerOutOfRange)?;
        let mut out = alloc::vec::Vec::new();
        self.read_exact_into(len, &mut out)?;
        if major == MAJOR_TYPE_TEXT_STRING {
            core::str::from_utf8(&out).map_err(Error::InvalidUtf8)?;
        }
        Ok(Some((Len::Known(len), alloc::borrow::Cow::Owned(out))))
    }

    #[inline(always)]
    fn read_break(&mut self) -> Result<()> {
        let byte = self.read_byte()?;
        if byte == BREAK {
            Ok(())
        } else {
            cold_path();
            invalid_initial_byte(byte)
        }
    }

    #[inline(always)]
    fn read_array_len(&mut self) -> Result<Len> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_ARRAY {
            cold_path();
            return invalid_initial_byte(byte);
        }
        // A count of 31 means "no count": the container ends at a break stop
        // code instead, which the caller's loop is what stops on.
        Ok(match self.read_additional_info(byte) {
            Ok(len) => Len::Known(usize::try_from(len).map_err(|_| Error::IntegerOutOfRange)?),
            Err(Error::InvalidAdditionalInfo(_)) => Len::Indefinite,
            Err(error) => return Err(error),
        })
    }

    #[inline(always)]
    fn read_map_len(&mut self) -> Result<Len> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_MAP {
            cold_path();
            return invalid_initial_byte(byte);
        }
        // A count of 31 means "no count": the container ends at a break stop
        // code instead, which the caller's loop is what stops on.
        Ok(match self.read_additional_info(byte) {
            Ok(len) => Len::Known(usize::try_from(len).map_err(|_| Error::IntegerOutOfRange)?),
            Err(Error::InvalidAdditionalInfo(_)) => Len::Indefinite,
            Err(error) => return Err(error),
        })
    }

    #[inline(always)]
    fn read_tag(&mut self) -> Result<u64> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_TAG {
            cold_path();
            return invalid_initial_byte(byte);
        }
        self.read_additional_info(byte)
    }

    #[inline(always)]
    fn read_string(&mut self) -> Result<alloc::borrow::Cow<'de, str>> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_TEXT_STRING {
            cold_path();
            return invalid_initial_byte(byte);
        }
        // Text chunks have already been checked independently by read_chunk.
        validate_utf8(alloc::borrow::Cow::Owned(self.read_string_head(byte)?))
    }

    #[inline(always)]
    fn read_string_bytes(&mut self) -> Result<alloc::borrow::Cow<'de, [u8]>> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_TEXT_STRING {
            cold_path();
            return invalid_initial_byte(byte);
        }
        Ok(alloc::borrow::Cow::Owned(self.read_string_head(byte)?))
    }

    #[inline(always)]
    fn read_binary(&mut self) -> Result<alloc::borrow::Cow<'de, [u8]>> {
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        if major_type != MAJOR_TYPE_BYTE_STRING {
            cold_path();
            return invalid_initial_byte(byte);
        }
        Ok(alloc::borrow::Cow::Owned(self.read_binary_head(byte)?))
    }

    #[inline(always)]
    fn read_option<T: FromCbor<'de>>(&mut self) -> Result<Option<T>> {
        let byte = self.peek_initial_byte()?;
        if byte == SIMPLE_VALUE_NULL {
            self.read_byte()?;
            Ok(None)
        } else {
            Ok(Some(T::read(self)?))
        }
    }

    fn skip_value(&mut self) -> Result<()> {
        self.increment_depth()?;
        let byte = self.read_byte()?;
        let major_type = byte & MAJOR_TYPE_MASK;
        let info = byte & !MAJOR_TYPE_MASK;

        match major_type {
            MAJOR_TYPE_UNSIGNED_INT | MAJOR_TYPE_NEGATIVE_INT | MAJOR_TYPE_TAG => {
                self.skip_additional_info(info)?;
            }
            MAJOR_TYPE_BYTE_STRING | MAJOR_TYPE_TEXT_STRING => {
                if info == ADDITIONAL_INFO_INDEFINITE {
                    // The chunks are walked rather than joined, so the bytes
                    // are never held in memory.
                    while self.read_chunk(major_type)?.is_some() {}
                } else {
                    let len = usize::try_from(self.skip_additional_info(info)?)
                        .map_err(|_| Error::IntegerOutOfRange)?;
                    let _ = self.read_exact_vec(len)?;
                }
            }
            MAJOR_TYPE_ARRAY => {
                if info == ADDITIONAL_INFO_INDEFINITE {
                    while !self.at_break()? {
                        self.skip_value()?;
                    }
                    self.read_break()?;
                } else {
                    let len = usize::try_from(self.skip_additional_info(info)?)
                        .map_err(|_| Error::IntegerOutOfRange)?;
                    for _ in 0..len {
                        self.skip_value()?;
                    }
                }
            }
            MAJOR_TYPE_MAP => {
                if info == ADDITIONAL_INFO_INDEFINITE {
                    while !self.at_break()? {
                        self.skip_value()?;
                        self.skip_value()?;
                    }
                    self.read_break()?;
                } else {
                    let len = usize::try_from(self.skip_additional_info(info)?)
                        .map_err(|_| Error::IntegerOutOfRange)?;
                    for _ in 0..len {
                        self.skip_value()?;
                        self.skip_value()?;
                    }
                }
            }
            MAJOR_TYPE_SIMPLE_FLOAT => match info {
                0..=23 | 31 => {}
                ADDITIONAL_INFO_1_BYTE => {
                    let mut buf = [0u8; 1];
                    self.read_exact(&mut buf)?;
                }
                ADDITIONAL_INFO_2_BYTES => {
                    let mut buf = [0u8; 2];
                    self.read_exact(&mut buf)?;
                }
                ADDITIONAL_INFO_4_BYTES => {
                    let mut buf = [0u8; 4];
                    self.read_exact(&mut buf)?;
                }
                ADDITIONAL_INFO_8_BYTES => {
                    let mut buf = [0u8; 8];
                    self.read_exact(&mut buf)?;
                }
                _ => {
                    cold_path();
                    return Err(Error::InvalidInitialByte(byte));
                }
            },
            _ => {
                cold_path();
                return Err(Error::InvalidInitialByte(byte));
            }
        }
        self.decrement_depth();
        Ok(())
    }
}

#[cfg(feature = "std")]
impl<R: std::io::Read> IOReader<R> {
    /// Reads exactly `len` bytes into `out`, replacing its contents.
    ///
    /// The buffer is grown once for the whole chunk and then filled, rather
    /// than one `read` call per byte.
    #[inline(never)]
    fn read_exact_into(&mut self, len: usize, out: &mut alloc::vec::Vec<u8>) -> Result<()> {
        const CHUNK_SIZE: usize = 8192;

        out.clear();
        if len == 0 {
            return Ok(());
        } else if len < CHUNK_SIZE {
            // Short enough for a stack buffer, so it is one read and one copy.
            let mut buf = [0u8; CHUNK_SIZE];
            self.reader
                .read_exact(&mut buf[..len])
                .map_err(Error::IoError)?;
            out.extend_from_slice(&buf[..len]);
            return Ok(());
        }

        // The length is off the wire, so nothing is reserved from it. A stream
        // cannot check a claim against the bytes that remain, and `try_reserve`
        // is no substitute: a nine-byte header asking for a hundred and fifty
        // gigabytes is a fine allocation. So the bytes are read in bounded steps
        // and the buffer grows only as they arrive, which caps it at what
        // arrived plus one step.
        let mut chunk = [0u8; CHUNK_SIZE];
        let mut remaining = len;
        while remaining > 0 {
            let to_read = core::cmp::min(remaining, chunk.len());
            let n = self
                .reader
                .read(&mut chunk[..to_read])
                .map_err(Error::IoError)?;
            if n == 0 {
                cold_path();
                return Err(Error::BufferTooSmall);
            }
            out.extend_from_slice(&chunk[..n]);
            remaining -= n;
        }
        Ok(())
    }

    #[inline(always)]
    fn skip_additional_info(&mut self, info: u8) -> Result<u64> {
        match info {
            0..=23 => Ok(info as u64),
            ADDITIONAL_INFO_1_BYTE => {
                let val = self.read_byte()? as u64;
                Ok(val)
            }
            ADDITIONAL_INFO_2_BYTES => {
                let mut bytes = [0; 2];
                self.read_exact(&mut bytes)?;
                Ok(u16::from_be_bytes(bytes) as u64)
            }
            ADDITIONAL_INFO_4_BYTES => {
                let mut bytes = [0; 4];
                self.read_exact(&mut bytes)?;
                Ok(u32::from_be_bytes(bytes) as u64)
            }
            ADDITIONAL_INFO_8_BYTES => {
                let mut bytes = [0; 8];
                self.read_exact(&mut bytes)?;
                Ok(u64::from_be_bytes(bytes))
            }
            ADDITIONAL_INFO_INDEFINITE => {
                cold_path();
                Err(Error::InvalidAdditionalInfo(ADDITIONAL_INFO_INDEFINITE))
            }
            _ => {
                cold_path();
                Err(Error::InvalidInitialByte(info))
            }
        }
    }
}
