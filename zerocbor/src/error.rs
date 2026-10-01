use core::fmt::Display;

/// Represents an error that can occur during CBOR encoding or decoding.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The initial byte is not a valid head, or names a major type this
    /// reader cannot handle.
    InvalidInitialByte(u8),
    /// A text string contained a character that is not valid UTF-8.
    InvalidChar,
    /// A text string's bytes are not valid UTF-8.
    InvalidUtf8(core::str::Utf8Error),
    /// A fixed-size output buffer ran out of room before the value was complete.
    BufferTooSmall,
    /// A borrow was requested from a reader that cannot lend out its buffer.
    CannotBorrow,
    /// An array's encoded length did not match the number of fields the target
    /// type has.
    ArrayLengthMismatch {
        /// The number of elements the target type requires.
        expected: usize,
        /// The number of elements the input declared.
        actual: usize,
    },
    /// A map's encoded length did not match the number of entries the type requires.
    MapLengthMismatch {
        /// The number of entries the target type requires.
        expected: usize,
        /// The number of entries the input declared.
        actual: usize,
    },
    /// A break stop code (`0xff`) appeared where no indefinite-length item was
    /// open.
    UnexpectedBreak,
    /// A value was untagged, or tagged with a different number, than the one
    /// its type declares. A value read from the wrong tag is a different value.
    TagMismatch {
        /// The tag the type declared.
        expected: u64,
        /// The tag that was on the wire, or `None` if the value was untagged.
        found: Option<u64>,
    },
    /// A value arrived under a tag its type does not declare. The mirror of
    /// [`Error::TagMismatch`].
    UnexpectedTag {
        /// The tag that was on the wire.
        found: u64,
    },
    /// A map held the same key twice.
    ///
    /// RFC 8949 Section 5.6 leaves this undefined and permits rejection.
    /// Keeping the last would make the result depend on the decoder.
    DuplicateKey,
    /// A floating-point value was decoded but does not fit the requested Rust
    /// type exactly.
    FloatNotExactlyRepresentable,
    /// An indefinite-length container was not closed by a break stop code.
    UnexpectedEndOfContainer {
        /// The byte that was found where the break should have been.
        byte: u8,
    },
    /// A head's additional information is not one RFC 8949 defines: `28`
    /// through `30` are unassigned, and `31` is a length, not a scalar.
    InvalidAdditionalInfo(u8),
    /// A float payload was truncated, or its marker was not a CBOR float width.
    InvalidFloatEncoding,
    /// A simple value below 32 arrived in the two-byte form, which RFC 8949
    /// Section 3.3 gives no meaning to.
    InvalidSimpleValue(u8),
    /// A CBOR integer was decoded but does not fit the requested Rust type.
    IntegerOutOfRange,
    /// The depth limit was exceeded while decoding a nested structure.
    DepthLimitExceeded,
    /// A map did not contain a key that the target type requires.
    KeyNotFound(alloc::string::String),
    /// A map contained the same key twice.
    KeyDuplicated(alloc::string::String),
    /// A map-mode enum found a variant name it does not know.
    UnknownVariant(alloc::string::String),
    /// An array-mode enum found a variant index it does not know.
    UnknownVariantIndex(u32),
    /// A `c_enum` integer did not match any Rust discriminant.
    UnknownVariantDiscriminant(i128),
    /// An underlying [`std::io`] operation failed.
    #[cfg(feature = "std")]
    IoError(std::io::Error),
}

/// The result of a CBOR encoding or decoding operation.
pub type Result<T> = core::result::Result<T, Error>;

impl Error {
    #[doc(hidden)]
    #[inline]
    pub fn key_not_found(key: &str) -> Self {
        Error::KeyNotFound(alloc::string::String::from(key))
    }

    #[doc(hidden)]
    #[inline]
    pub fn key_duplicated(key: &str) -> Self {
        Error::KeyDuplicated(alloc::string::String::from(key))
    }

    #[doc(hidden)]
    #[inline]
    pub fn unknown_variant(name: &str) -> Self {
        Error::UnknownVariant(alloc::string::String::from(name))
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::InvalidInitialByte(byte) => write!(f, "Invalid initial byte: 0x{:02x}", byte),
            Error::InvalidChar => write!(f, "Invalid character"),
            Error::InvalidUtf8(err) => write!(f, "Invalid UTF-8: {}", err),
            Error::BufferTooSmall => write!(f, "Buffer too small"),
            Error::CannotBorrow => write!(f, "Cannot borrow data from original buffer"),
            Error::ArrayLengthMismatch { expected, actual } => {
                write!(
                    f,
                    "Array length mismatch: expected {}, actual {}",
                    expected, actual
                )
            }
            Error::MapLengthMismatch { expected, actual } => {
                write!(
                    f,
                    "Map length mismatch: expected {}, actual {}",
                    expected, actual
                )
            }
            Error::UnexpectedBreak => write!(f, "Unexpected break code"),
            Error::TagMismatch { expected, found } => match found {
                Some(found) => write!(f, "Expected tag {expected} but the value is tagged {found}"),
                None => write!(f, "Expected tag {expected} but the value is not tagged"),
            },
            Error::UnexpectedTag { found } => {
                write!(f, "Value is tagged {found} but its type declares no tag")
            }
            Error::DuplicateKey => write!(f, "Map has a duplicate key"),
            Error::FloatNotExactlyRepresentable => {
                write!(f, "Float is not exactly representable in the target type")
            }
            Error::UnexpectedEndOfContainer { byte } => write!(
                f,
                "Indefinite-length container was not closed by a break code, found 0x{:02x}",
                byte
            ),
            Error::InvalidAdditionalInfo(info) => {
                write!(f, "Invalid additional information: {}", info)
            }
            Error::InvalidFloatEncoding => write!(f, "Invalid floating-point encoding"),
            Error::InvalidSimpleValue(value) => write!(f, "Invalid simple value: {}", value),
            Error::IntegerOutOfRange => write!(f, "Integer out of range for the target type"),
            Error::DepthLimitExceeded => {
                write!(f, "Maximum deserialization depth exceeded")
            }
            Error::KeyNotFound(key) => write!(f, "Key '{}' not found", key),
            Error::KeyDuplicated(key) => write!(f, "Key '{}' is duplicated", key),
            Error::UnknownVariant(name) => write!(f, "Unknown variant '{}'", name),
            Error::UnknownVariantIndex(index) => write!(f, "Unknown variant index {}", index),
            Error::UnknownVariantDiscriminant(value) => {
                write!(f, "Unknown variant discriminant {}", value)
            }
            #[cfg(feature = "std")]
            Error::IoError(err) => err.fmt(f),
        }
    }
}

impl core::error::Error for Error {}
