use core::cmp::Ordering;
use core::hint::cold_path;

use crate::consts::MAJOR_TYPE_MASK;
use crate::read::Len;
use crate::{Error, FromCbor, Read, Result, ToCbor, Write};
use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// A dynamically-typed CBOR value.
///
/// `#[non_exhaustive]`: CBOR's shapes are open, so a downstream `match` over
/// every variant would need editing for each addition here.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Value<'a> {
    /// Major type 7, the `null` simple value.
    Null,
    /// Major type 7, `true` or `false`.
    Bool(bool),
    /// Major type 7, simple value 23: `undefined`. Distinct from
    /// [`Value::Null`], which says a value is absent.
    Undefined,
    /// Major type 7, a simple value with no meaning of its own. RFC 8949
    /// Section 3.3 reserves these, so they are carried rather than rejected.
    Simple(u8),
    /// Major type 0 or 1: a non-negative or negative integer.
    Integer(i128),
    /// Major type 7, a half-, single-, or double-precision float.
    Float(f64),
    /// Major type 2, a byte string.
    Bytes(Cow<'a, [u8]>),
    /// Major type 3, a UTF-8 text string.
    Text(Cow<'a, str>),
    /// Major type 4, an array of values.
    Array(Vec<Value<'a>>),
    /// Major type 5, a map from values to values.
    Map(BTreeMap<Value<'a>, Value<'a>>),
    /// Major type 6, a tagged value: the tag number and the value it applies to.
    Tag(u64, Box<Value<'a>>),
}

impl<'a> Eq for Value<'a> {}

impl<'a> PartialEq for Value<'a> {
    /// A `NaN` equals itself here, unlike the `f64` it rides in.
    ///
    /// A CBOR `NaN` has no payload a decoder must keep: RFC 8949 Section 4.1
    /// names one encoding and Section 4.2.1 requires it. Leaving the derived
    /// comparison would make `Eq` unsound and a `BTreeMap<Value, _>` would
    /// treat each `NaN` as a fresh key.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Float(a), Value::Float(b)) => a == b || (a.is_nan() && b.is_nan()),
            (Value::Tag(a_tag, a), Value::Tag(b_tag, b)) => a_tag == b_tag && a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::Text(a), Value::Text(b)) => a == b,
            _ => {
                core::mem::discriminant(self) == core::mem::discriminant(other)
                    && match (self, other) {
                        (Value::Null, Value::Null) | (Value::Undefined, Value::Undefined) => true,
                        (Value::Bool(a), Value::Bool(b)) => a == b,
                        (Value::Simple(a), Value::Simple(b)) => a == b,
                        (Value::Integer(a), Value::Integer(b)) => a == b,
                        _ => false,
                    }
            }
        }
    }
}

impl<'a> PartialOrd for Value<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> Ord for Value<'a> {
    /// The order RFC 8949 Section 4.2.1 defines for map keys: bytewise over
    /// their deterministic encodings.
    ///
    /// A `BTreeMap` iterates in this order, so a map of values comes out
    /// deterministic with no sorting at write time. Heads go first, which
    /// settles major type and float width without walking contents; equal heads
    /// mean the same shape, since CBOR is self-delimiting, so the rest is
    /// content.
    fn cmp(&self, other: &Self) -> Ordering {
        let mut left = [0u8; MAX_HEAD];
        let mut right = [0u8; MAX_HEAD];
        let (left, right) = (self.encoded_head(&mut left), other.encoded_head(&mut right));
        match left.cmp(right) {
            Ordering::Equal => self.cmp_content(other),
            ordering => ordering,
        }
    }
}

/// The largest encoded head: an initial byte and an eight-byte argument.
const MAX_HEAD: usize = 9;

impl<'a> Value<'a> {
    /// Writes the encoded head into `out` and returns the part that is the head.
    ///
    /// The head is what orders two values, so for a float it is the narrowed
    /// width, not the `f64` one.
    fn encoded_head<'o>(&self, out: &'o mut [u8; MAX_HEAD]) -> &'o [u8] {
        let len = match self {
            Value::Null => {
                out[0] = crate::consts::SIMPLE_VALUE_NULL;
                1
            }
            Value::Bool(value) => {
                out[0] = if *value {
                    crate::consts::SIMPLE_VALUE_TRUE
                } else {
                    crate::consts::SIMPLE_VALUE_FALSE
                };
                1
            }
            Value::Undefined => {
                out[0] = crate::consts::SIMPLE_VALUE_UNDEFINED;
                1
            }
            Value::Simple(value) => {
                if *value <= 23 {
                    out[0] = crate::consts::MAJOR_TYPE_SIMPLE_FLOAT | value;
                    1
                } else {
                    out[0] = crate::consts::MAJOR_TYPE_SIMPLE_FLOAT | 24;
                    out[1] = *value;
                    2
                }
            }
            Value::Integer(value) => {
                let argument = integer_argument(*value).unwrap_or(0);
                if *value >= 0 {
                    out[0] = crate::consts::MAJOR_TYPE_UNSIGNED_INT;
                } else {
                    out[0] = crate::consts::MAJOR_TYPE_NEGATIVE_INT;
                }
                write_head(out, argument);
                argument_head_len(argument) + 1
            }
            Value::Float(value) => {
                out[0] = crate::r#impl::preferred_float_marker(*value);
                1
            }
            Value::Bytes(bytes) => {
                out[0] = crate::consts::MAJOR_TYPE_BYTE_STRING;
                write_head(out, bytes.len() as u64);
                argument_head_len(bytes.len() as u64) + 1
            }
            Value::Text(text) => {
                out[0] = crate::consts::MAJOR_TYPE_TEXT_STRING;
                write_head(out, text.len() as u64);
                argument_head_len(text.len() as u64) + 1
            }
            Value::Array(items) => {
                out[0] = crate::consts::MAJOR_TYPE_ARRAY;
                write_head(out, items.len() as u64);
                argument_head_len(items.len() as u64) + 1
            }
            Value::Map(entries) => {
                out[0] = crate::consts::MAJOR_TYPE_MAP;
                write_head(out, entries.len() as u64);
                argument_head_len(entries.len() as u64) + 1
            }
            Value::Tag(tag, _) => {
                out[0] = crate::consts::MAJOR_TYPE_TAG;
                write_head(out, *tag);
                argument_head_len(*tag) + 1
            }
        };
        &out[..len]
    }

    /// The order of two values whose encoded heads are equal, so what is left is
    /// the payload, compared as its own encoding would be.
    fn cmp_content(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            // By its encoding, not its value: that is this type's documented
            // order and a deterministic writer's. See `float_sort_key`.
            (Value::Float(a), Value::Float(b)) => float_sort_key(*a).cmp(&float_sort_key(*b)),
            (Value::Bytes(a), Value::Bytes(b)) => a.cmp(b),
            (Value::Text(a), Value::Text(b)) => a.cmp(b),
            (Value::Array(a), Value::Array(b)) => a.cmp(b),
            (Value::Map(a), Value::Map(b)) => a.iter().cmp(b.iter()),
            (Value::Tag(a_tag, a), Value::Tag(b_tag, b)) => a_tag.cmp(b_tag).then_with(|| a.cmp(b)),
            (Value::Null, Value::Null)
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Undefined, Value::Undefined)
            | (Value::Simple(_), Value::Simple(_)) => Ordering::Equal,
            // Two variants can share a head only for simple values 20 to 23,
            // which have their own. The order is still total, so the declaration
            // order settles it.
            _ => variant_rank(self).cmp(&variant_rank(other)),
        }
    }
}

/// The argument a CBOR integer puts on the wire, or `None` if it has none.
///
/// The argument is at most 64 bits, so a wider `i128` has no encoding and is
/// reported rather than truncated. The negative form is the bitwise complement,
/// which is `-1 - value` without the overflow `i128::MIN` would cause.
fn integer_argument(value: i128) -> Option<u64> {
    if value >= 0 {
        u64::try_from(value).ok()
    } else {
        u64::try_from(!value).ok()
    }
}

/// A key whose order is the bytewise order of the value's encoding.
///
/// An encoding is a marker byte then a big-endian payload, so the order is the
/// width first and then the payload as an integer: `0xf9` before `0xfa` and
/// `0xfb`. Ordering by the float's value would not do — a double sorts after
/// every half however small it is, and a `NaN` has no value.
#[inline]
fn float_sort_key(value: f64) -> (u8, u64) {
    use crate::consts::{FLOAT16_MARKER, FLOAT32_MARKER};
    if value.is_nan() {
        // `0xf97e00`, a `binary16`, so it sits among the halves: after `+inf`
        // at `0x7c00`, before `-0.0` at `0x8000`.
        return (FLOAT16_MARKER, 0x7e00);
    }
    match crate::r#impl::preferred_float_marker(value) {
        FLOAT16_MARKER => (
            FLOAT16_MARKER,
            crate::write::encode_f16(value as f32) as u64,
        ),
        FLOAT32_MARKER => (FLOAT32_MARKER, (value as f32).to_bits() as u64),
        _ => (crate::consts::FLOAT64_MARKER, value.to_bits()),
    }
}

/// The declaration index of a variant, used only to break a tie the encoded
/// heads cannot.
fn variant_rank(value: &Value<'_>) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Undefined => 2,
        Value::Simple(_) => 3,
        Value::Integer(_) => 4,
        Value::Float(_) => 5,
        Value::Bytes(_) => 6,
        Value::Text(_) => 7,
        Value::Array(_) => 8,
        Value::Map(_) => 9,
        Value::Tag(_, _) => 10,
    }
}

/// Writes the argument into `out`, big-endian, from index 1 on: the initial
/// byte is already there.
fn write_head(out: &mut [u8; MAX_HEAD], argument: u64) {
    use crate::consts::ADDITIONAL_INFO_1_BYTE as U8;
    use crate::consts::ADDITIONAL_INFO_2_BYTES as U16;
    use crate::consts::ADDITIONAL_INFO_4_BYTES as U32;
    match argument {
        0..=23 => out[0] |= argument as u8,
        24..=0xff => {
            out[0] |= U8;
            out[1] = argument as u8;
        }
        0x100..=0xffff => {
            out[0] |= U16;
            out[1..3].copy_from_slice(&(argument as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out[0] |= U32;
            out[1..5].copy_from_slice(&(argument as u32).to_be_bytes());
        }
        _ => {
            out[0] |= crate::consts::ADDITIONAL_INFO_8_BYTES;
            out[1..9].copy_from_slice(&argument.to_be_bytes());
        }
    }
}

/// The bytes an argument takes: the additional information plus its payload.
fn argument_head_len(argument: u64) -> usize {
    match argument {
        0..=23 => 0,
        24..=0xff => 1,
        0x100..=0xffff => 2,
        0x1_0000..=0xffff_ffff => 4,
        _ => 8,
    }
}

impl<'a> FromCbor<'a> for Value<'a> {
    fn read<R: Read<'a>>(reader: &mut R) -> Result<Self> {
        read_value(reader)
    }
}

fn read_value<'a, R: Read<'a>>(reader: &mut R) -> Result<Value<'a>> {
    let byte = reader.peek_initial_byte()?;

    match byte & MAJOR_TYPE_MASK {
        crate::consts::MAJOR_TYPE_ARRAY => {
            let len = reader.read_array_len()?;
            let mut arr = Vec::new();
            // The count is off the wire, so cap it: a 6-byte header would
            // otherwise reserve gigabytes. An indefinite array has no count.
            if let Len::Known(count) = len {
                arr.reserve(count.min(1024));
            }
            let mut iter = reader.array_iter_from(Some(len))?;
            while iter.next(reader)? {
                match read_value(reader) {
                    Ok(value) => arr.push(value),
                    Err(error) => {
                        reader.decrement_depth();
                        return Err(error);
                    }
                }
            }
            reader.finish_array(len)?;
            Ok(Value::Array(arr))
        }
        crate::consts::MAJOR_TYPE_MAP => {
            let len = reader.read_map_len()?;
            let mut map = BTreeMap::new();
            let mut iter = reader.map_iter_from(Some(len))?;
            while iter.next(reader)? {
                let key = match read_value(reader) {
                    Ok(key) => key,
                    Err(error) => {
                        reader.decrement_depth();
                        return Err(error);
                    }
                };
                let value = match read_value(reader) {
                    Ok(value) => value,
                    Err(error) => {
                        reader.decrement_depth();
                        return Err(error);
                    }
                };
                // RFC 8949 Section 5.6 leaves this undefined; the derive
                // rejects one and this agrees with it.
                if map.insert(key, value).is_some() {
                    cold_path();
                    reader.decrement_depth();
                    return Err(Error::DuplicateKey);
                }
            }
            reader.finish_map(len)?;
            Ok(Value::Map(map))
        }
        crate::consts::MAJOR_TYPE_TAG => {
            let tag = reader.read_tag()?;
            reader.increment_depth()?;
            let value = match read_value(reader) {
                Ok(value) => value,
                Err(error) => {
                    reader.decrement_depth();
                    return Err(error);
                }
            };
            reader.decrement_depth();
            Ok(Value::Tag(tag, Box::new(value)))
        }
        // Everything else is a leaf, so no recursion.
        _ => read_leaf(reader, byte),
    }
}

fn read_leaf<'a, R: Read<'a>>(reader: &mut R, byte: u8) -> Result<Value<'a>> {
    match byte & MAJOR_TYPE_MASK {
        // Both go through the wide reader: a type 0 argument reaches
        // `u64::MAX`, and a type 1 one is a bit wider still.
        crate::consts::MAJOR_TYPE_UNSIGNED_INT | crate::consts::MAJOR_TYPE_NEGATIVE_INT => {
            Ok(Value::Integer(reader.read_integer()?))
        }
        crate::consts::MAJOR_TYPE_BYTE_STRING => Ok(Value::Bytes(reader.read_binary()?)),
        crate::consts::MAJOR_TYPE_TEXT_STRING => Ok(Value::Text(reader.read_string()?)),
        crate::consts::MAJOR_TYPE_SIMPLE_FLOAT => {
            // These are complete initial bytes, so they match `byte`; matching
            // the masked `info` would compare 0xf5 against 0x05 and never hit.
            match byte {
                crate::consts::SIMPLE_VALUE_FALSE => {
                    reader.read_boolean()?;
                    Ok(Value::Bool(false))
                }
                crate::consts::SIMPLE_VALUE_TRUE => {
                    reader.read_boolean()?;
                    Ok(Value::Bool(true))
                }
                crate::consts::SIMPLE_VALUE_NULL => {
                    reader.read_null()?;
                    Ok(Value::Null)
                }
                crate::consts::SIMPLE_VALUE_UNDEFINED => {
                    reader.read_undefined()?;
                    Ok(Value::Undefined)
                }
                crate::consts::FLOAT16_MARKER => Ok(Value::Float(reader.read_f16()? as f64)),
                crate::consts::FLOAT32_MARKER => Ok(Value::Float(reader.read_f32()? as f64)),
                crate::consts::FLOAT64_MARKER => Ok(Value::Float(reader.read_f64()?)),
                // The rest of major type 7: 0 through 19, and 32 through
                // 255 in the two-byte form.
                _ => Ok(Value::Simple(reader.read_simple_value()?)),
            }
        }
        _ => Err(Error::InvalidInitialByte(byte)),
    }
}

impl<'a> ToCbor for Value<'a> {
    fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        match self {
            Value::Null => writer.write_null(),
            Value::Bool(b) => writer.write_boolean(*b),
            Value::Undefined => writer.write_undefined(),
            Value::Simple(v) => writer.write_simple_value(*v),
            Value::Integer(i) => {
                // No encoding for a wider `i128`; reported, not truncated.
                let argument = integer_argument(*i).ok_or(Error::IntegerOutOfRange)?;
                if *i >= 0 {
                    writer.write_u64(argument)
                } else {
                    writer.write_negative(argument)
                }
            }
            Value::Float(f) => (*f).write(writer),
            Value::Bytes(b) => writer.write_binary(b),
            Value::Text(t) => writer.write_string(t),
            Value::Array(arr) => {
                writer.write_array_len(arr.len())?;
                for item in arr {
                    item.write(writer)?;
                }
                Ok(())
            }
            Value::Map(map) => {
                writer.write_map_len(map.len())?;
                for (k, v) in map {
                    k.write(writer)?;
                    v.write(writer)?;
                }
                Ok(())
            }
            Value::Tag(tag, value) => {
                writer.write_tag(*tag)?;
                value.write(writer)
            }
        }
    }

    fn size_hint(&self) -> Option<crate::TrustedSizeHint> {
        match self {
            Value::Null | Value::Bool(_) | Value::Undefined => {
                Some(unsafe { crate::TrustedSizeHint::new_unchecked(1) })
            }
            // A simple value above 23 needs the two-byte form.
            Value::Simple(v) => {
                Some(unsafe { crate::TrustedSizeHint::new_unchecked(if *v <= 23 { 1 } else { 2 }) })
            }
            // An `i128` with no encoding has no size; the write path reports it.
            Value::Integer(i) => integer_argument(*i).and_then(|argument| argument.size_hint()),
            // Preferred serialization, so the width depends on the value.
            Value::Float(f) => f.size_hint(),
            Value::Bytes(b) => crate::r#impl::string_size_hint(b.len()),
            Value::Text(t) => crate::r#impl::string_size_hint(t.len()),
            // A size hint must be O(1). Traversing containers would encode
            // every dynamic document twice before writing its first byte.
            Value::Array(_) | Value::Map(_) | Value::Tag(_, _) => None,
        }
    }
}

impl<'a> Value<'a> {
    /// Returns `true` if the value is `Null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Returns `true` if the value is a boolean.
    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    /// Returns `true` if the value is an integer.
    pub fn is_integer(&self) -> bool {
        matches!(self, Value::Integer(_))
    }

    /// Returns `true` if the value is a float.
    pub fn is_float(&self) -> bool {
        matches!(self, Value::Float(_))
    }

    /// Returns `true` if the value is a byte string.
    pub fn is_bytes(&self) -> bool {
        matches!(self, Value::Bytes(_))
    }

    /// Returns `true` if the value is a text string.
    pub fn is_text(&self) -> bool {
        matches!(self, Value::Text(_))
    }

    /// Returns `true` if the value is an array.
    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    /// Returns `true` if the value is a map.
    pub fn is_map(&self) -> bool {
        matches!(self, Value::Map(_))
    }

    /// Returns `true` if the value is a tag.
    pub fn is_tag(&self) -> bool {
        matches!(self, Value::Tag(_, _))
    }

    /// Returns the value as a boolean if it is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Returns the value as an integer if it is one.
    pub fn as_integer(&self) -> Option<i128> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Returns the value as a float if it is one.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Returns the value as a byte slice if it is a byte string.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Returns the value as a string slice if it is a text string.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Returns the value as a slice if it is an array.
    pub fn as_array(&self) -> Option<&[Value<'a>]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Returns the value as a map if it is one.
    pub fn as_map(&self) -> Option<&BTreeMap<Value<'a>, Value<'a>>> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    /// Returns the tag value and inner value if it is a tag.
    pub fn as_tag(&self) -> Option<(u64, &Value<'a>)> {
        match self {
            Value::Tag(tag, value) => Some((*tag, value)),
            _ => None,
        }
    }
}
