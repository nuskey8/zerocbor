#![no_std]
#![doc = include_str!("../README.md")]
// `f16` is unstable, so the feature needs nightly. Off by default: a build that
// did not ask for half-precision floats must not require it.
#![cfg_attr(feature = "f16", feature(f16))]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

#[cfg(test)]
extern crate self as zerocbor;

mod consts;
mod error;
mod r#impl;
mod read;
mod value;
mod write;

use alloc::vec::Vec;

pub use consts::tags;
pub use error::{Error, Result};
pub use read::{ArrayIter, Len, MAX_DEPTH, MapIter, Read};
pub use value::Value;
pub use write::Write;

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

#[cfg(feature = "derive")]
pub use zerocbor_derive::{FromCbor, ToCbor};

/// A data structure that can be deserialized from CBOR format.
pub trait FromCbor<'a>
where
    Self: Sized,
{
    /// Reads the CBOR representation of this value from the provided reader.
    fn read<R: Read<'a>>(reader: &mut R) -> Result<Self>;
}

/// A trait for types that can be deserialized from CBOR format without borrowing.
pub trait FromCborOwned: for<'a> FromCbor<'a> {}

impl<T> FromCborOwned for T where T: for<'a> FromCbor<'a> {}

/// An encoded-size hint used for preallocation, never for memory safety.
pub struct TrustedSizeHint(usize);

impl TrustedSizeHint {
    /// Creates a trusted encoded-size upper bound.
    ///
    /// # Safety
    /// The next serialization of this value must write at most `upper_bound`
    /// bytes, and serialization-relevant state must not change in between.
    #[doc(hidden)]
    #[inline(always)]
    pub const unsafe fn new_unchecked(upper_bound: usize) -> Self {
        Self(upper_bound)
    }

    /// Returns the bound this hint was built with: a ceiling, not a prediction.
    /// A shorter encoding is fine, a longer one a contract violation.
    #[inline(always)]
    pub const fn upper_bound(&self) -> usize {
        self.0
    }
}

/// A data structure that can be serialized into CBOR format.
pub trait ToCbor {
    /// Writes the CBOR representation of this value into the provided writer.
    fn write<W: Write>(&self, writer: &mut W) -> Result<()>;

    /// Writes the CBOR representation of a slice of values into the provided writer.
    #[inline(always)]
    fn write_slice<W: Write>(values: &[Self], writer: &mut W) -> Result<()>
    where
        Self: Sized,
    {
        for value in values {
            value.write(writer)?;
        }
        Ok(())
    }

    /// Returns a trusted upper bound on the bytes the next [`Self::write`] will
    /// write, or `None` if it cannot be had cheaply.
    ///
    /// Must run in O(1) in the value's runtime-sized contents: return `None`
    /// rather than traverse a slice, string or collection.
    ///
    /// The bound should cover the encoding. Writers still check their bounds,
    /// so an incorrect hint cannot cause an out-of-bounds write.
    #[inline]
    fn size_hint(&self) -> Option<TrustedSizeHint> {
        None
    }

    /// Returns a trusted upper bound valid for every value of this type, for
    /// O(1) hints on runtime-sized homogeneous containers.
    ///
    /// The bound should cover every encoding of this type. It is used only
    /// for preallocation; writers still check their bounds.
    #[inline]
    fn max_size() -> Option<TrustedSizeHint>
    where
        Self: Sized,
    {
        None
    }
}

/// Checks the tag a value arrived under against the one its type declares.
///
/// `None` for `expected` means the type declares no tag and requires the value
/// to be untagged. Reporting a mismatch rather than reading anyway is what stops
/// a tagged value being reinterpreted as a different one.
///
/// `#[cbor(tag = N)]` generates a call to this, so a hand-written `FromCbor`
/// that declares a tag should too.
pub fn check_optional_tag(expected: Option<u64>, found: Option<u64>) -> Result<()> {
    if expected == found {
        return Ok(());
    }
    Err(match (expected, found) {
        (Some(expected), Some(found)) => Error::TagMismatch {
            expected,
            found: Some(found),
        },
        (Some(expected), None) => Error::TagMismatch {
            expected,
            found: None,
        },
        (None, Some(found)) => Error::UnexpectedTag { found },
        (None, None) => unreachable!("the options are equal"),
    })
}

/// Deserializes a `T` from a CBOR-encoded byte slice.
///
/// Fails only if `T`'s [`FromCbor`] does.
///
/// ```
/// let cbor = vec![0x01]; // unsigned integer 1
/// let value: u64 = zerocbor::from_cbor(&cbor).unwrap();
/// assert_eq!(value, 1);
/// ```
pub fn from_cbor<'a, T: FromCbor<'a>>(data: &'a [u8]) -> Result<T> {
    let mut reader = read::SliceReader::new(data);
    T::read(&mut reader)
}

/// Serializes a `T` into a `Vec<u8>`.
///
/// Fails only if `T`'s [`ToCbor`] does.
///
/// ```
/// let value: u64 = 1;
/// let cbor: Vec<u8> = zerocbor::to_cbor_vec(&value).unwrap();
/// assert_eq!(cbor, vec![0x01]);
/// ```
pub fn to_cbor_vec<T: ToCbor + ?Sized>(value: &T) -> Result<Vec<u8>> {
    /// Cap on size-hint preallocation, so a large bound does not over-allocate.
    const MAX_SIZE_HINT_PREALLOC: usize = 16 * 1024 * 1024;

    let mut writer = match value.size_hint() {
        Some(hint) => {
            write::VecWriter::with_capacity_hint(hint.upper_bound().min(MAX_SIZE_HINT_PREALLOC))
        }
        None => write::VecWriter::new(),
    };
    value.write(&mut writer)?;
    Ok(writer.into_vec())
}

/// Serializes a `T` into `buf`, returning the bytes written.
///
/// Also fails with [`Error::BufferTooSmall`] if `buf` is too small.
///
/// ```
/// let value: u64 = 1;
/// let mut buf = [0u8; 10];
/// let written = zerocbor::to_cbor(&value, &mut buf).unwrap();
/// assert_eq!(written, 1);
/// assert_eq!(&buf[..written], &[0x01]);
/// ```
pub fn to_cbor<T: ToCbor + ?Sized>(value: &T, buf: &mut [u8]) -> Result<usize> {
    let mut writer = write::SliceWriter::new(buf);
    value.write(&mut writer)?;
    Ok(writer.position())
}

/// Serializes a `T` into an [`std::io::Write`].
///
/// Also fails with [`Error::IoError`] if the stream does.
///
/// ```
/// let value: u64 = 1;
/// let mut buf = Vec::new();
/// zerocbor::write_cbor(&mut std::io::Cursor::new(&mut buf), &value).unwrap();
/// ```
#[cfg(feature = "std")]
pub fn write_cbor<T: ToCbor + ?Sized, W: std::io::Write>(writer: &mut W, value: &T) -> Result<()> {
    let mut io_writer = write::IOWriter::new(writer);
    value.write(&mut io_writer)
}

/// Deserializes a `T` from an [`std::io::Read`].
///
/// Also fails with [`Error::IoError`] if the stream does.
///
/// ```
/// let value: u64 = zerocbor::read_cbor(std::io::Cursor::new(vec![0x01])).unwrap();
/// assert_eq!(value, 1);
/// ```
#[cfg(feature = "std")]
pub fn read_cbor<'a, R: std::io::Read, T: FromCbor<'a>>(reader: R) -> Result<T> {
    let mut io_reader = read::IOReader::new(reader);
    T::read(&mut io_reader)
}
