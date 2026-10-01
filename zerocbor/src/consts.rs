pub const MAJOR_TYPE_UNSIGNED_INT: u8 = 0x00;
pub const MAJOR_TYPE_NEGATIVE_INT: u8 = 0x20;
pub const MAJOR_TYPE_BYTE_STRING: u8 = 0x40;
pub const MAJOR_TYPE_TEXT_STRING: u8 = 0x60;
pub const MAJOR_TYPE_ARRAY: u8 = 0x80;
pub const MAJOR_TYPE_MAP: u8 = 0xa0;
pub const MAJOR_TYPE_TAG: u8 = 0xc0;
pub const MAJOR_TYPE_SIMPLE_FLOAT: u8 = 0xe0;

pub const MAJOR_TYPE_MASK: u8 = 0xe0;

pub const ADDITIONAL_INFO_1_BYTE: u8 = 24;
pub const ADDITIONAL_INFO_2_BYTES: u8 = 25;
pub const ADDITIONAL_INFO_4_BYTES: u8 = 26;
pub const ADDITIONAL_INFO_8_BYTES: u8 = 27;
pub const ADDITIONAL_INFO_INDEFINITE: u8 = 31;

pub const SIMPLE_VALUE_FALSE: u8 = 0xf4;
pub const SIMPLE_VALUE_TRUE: u8 = 0xf5;
pub const SIMPLE_VALUE_NULL: u8 = 0xf6;
pub const SIMPLE_VALUE_UNDEFINED: u8 = 0xf7;

pub const FLOAT16_MARKER: u8 = 0xf9;
pub const FLOAT32_MARKER: u8 = 0xfa;
pub const FLOAT64_MARKER: u8 = 0xfb;

/// The break stop code, which closes an indefinite-length container or string.
pub const BREAK: u8 = 0xff;

/// The tag numbers RFC 8949 Section 3.4 and RFC 8746 assign meaning to.
///
/// A tag on the wire is just a number, so these are the names for them rather
/// than a decoder: nothing here interprets the tagged value. A type that wants
/// one says so with `#[cbor(tag = N)]`, which writes the number and checks it on
/// the way back in.
pub mod tags {
    /// Tag 0: an RFC 3339 date/time string, as defined by RFC 3339.
    pub const DATE_TIME_STRING: u64 = 0;
    /// Tag 1: an epoch-based date/time, either a number of seconds or a
    /// floating-point fraction of a second since 1970-01-01T00:00:00Z.
    pub const EPOCH_DATE_TIME: u64 = 1;
    /// Tag 2: a bignum, a byte string holding a non-negative integer in
    /// big-endian base-256 with no leading zeroes.
    pub const BIGNUM_UNSIGNED: u64 = 2;
    /// Tag 3: a negative bignum, held the same way as tag 2 but denoting `-1 - n`.
    pub const BIGNUM_NEGATIVE: u64 = 3;
    /// Tag 4: a decimal fraction, a two-element array of an exponent and a
    /// mantissa.
    pub const DECIMAL_FRACTION: u64 = 4;
    /// Tag 5: a bigfloat, a two-element array of an exponent and a mantissa.
    pub const BIGFLOAT: u64 = 5;
    /// Tag 24: an embedded CBOR data item, held as a byte string.
    pub const ENCODED_CBOR: u64 = 24;
    /// Tag 32: a URI, as defined by RFC 3986.
    pub const URI: u64 = 32;
    /// Tag 33: base64url-encoded data, as defined by RFC 4648 Section 5.
    pub const BASE64_URL: u64 = 33;
    /// Tag 34: base64-encoded data, as defined by RFC 4648 Section 4.
    pub const BASE64: u64 = 34;
    /// Tag 35: base16-encoded data, as defined by RFC 4648 Section 8.
    pub const BASE16: u64 = 35;
    /// Tag 64-87 are typed arrays, whose type is `tag - 64` in the array's own
    /// number space; see RFC 8746.
    pub const TYPED_ARRAY_START: u64 = 64;
    /// The first tag number that is not a typed array.
    pub const TYPED_ARRAY_END: u64 = 88;
    /// Tag 258: a set, which as a data model is an array whose members are
    /// distinct.
    pub const SET: u64 = 258;
    /// Tag 55799: self-described CBOR, the tag RFC 8949 Appendix D defines so a
    /// decoder can tell CBOR from another format that starts the same way.
    pub const SELF_DESCRIBED_CBOR: u64 = 55799;
}
