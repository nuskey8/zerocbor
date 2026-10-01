//! Types and data shared by the benchmarks.
//!
//! Every benchmark is its own crate, so each one compiles this module afresh and
//! only uses the parts it needs. That makes the unused items here expected.
#![allow(dead_code)]

use std::borrow::Cow;

use ciborium::value::Value as CiboriumValue;
use serde::{Deserialize, Serialize};
use zerocbor_derive::{FromCbor, ToCbor};

/// A fixed two-field record, the simplest thing worth measuring.
#[derive(ToCbor, FromCbor, Serialize, Deserialize, minicbor::Encode, minicbor::Decode)]
pub struct Point {
    #[n(0)]
    pub x: i32,
    #[n(1)]
    pub y: i32,
}

/// A four-field record with a string, a nested record, an optional, and a
/// variable-length list. The string and list make it representative of real
/// payloads rather than a run of machine words.
#[derive(ToCbor, FromCbor, Serialize, Deserialize, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
#[allow(dead_code)]
pub struct Nested {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub p1: Point,
    #[n(2)]
    pub p2: Option<Point>,
    #[n(3)]
    pub params: Vec<i32>,
}

/// The same shape written positionally instead of by key.
#[derive(ToCbor, FromCbor, Serialize, Deserialize, minicbor::Encode, minicbor::Decode)]
#[cbor(array)]
#[allow(dead_code)]
pub struct NestedArray {
    #[n(0)]
    pub name: String,
    #[n(1)]
    pub p1: Point,
    #[n(2)]
    pub p2: Option<Point>,
    #[n(3)]
    pub params: Vec<i32>,
}

/// A record whose fields borrow from the input, so decoding copies nothing.
///
/// The byte-string field is `Cow<[u8]>` with `#[cbor(as_bytes = true)]`, which is how a
/// byte string is asked for: a bare `&[u8]` would be an array of integers. It
/// decodes to `Cow::Borrowed` when the input is a slice.
///
/// `minicbor` is deliberately absent: 2.x borrows a byte string only as `&str`,
/// so it has no `Decode` impl for a byte-string field and cannot be measured on
/// this type.
#[derive(ToCbor, FromCbor)]
pub struct NoCopy<'a> {
    pub str: &'a str,
    #[cbor(as_bytes = true)]
    pub bin: Cow<'a, [u8]>,
}

/// The value every zero-copy benchmark uses for the text field.
pub const SAMPLE_STR: &str = "hello, world!!";
/// The value every zero-copy benchmark uses for the byte-string field.
pub const SAMPLE_BIN: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 0];

/// The serde-facing twin of [`NoCopy`].
#[derive(Serialize, Deserialize)]
pub struct NoCopySerde<'a> {
    pub str: &'a str,
    #[serde(with = "serde_bytes")]
    pub bin: &'a [u8],
}

impl NoCopySerde<'static> {
    /// The same bytes as [`NoCopy`], in the form the serde-based encoders need.
    ///
    /// `serde_bytes` is what makes serde write a byte string instead of an array,
    /// so this type round-trips through `ciborium` and `cbor4ii`.
    pub fn sample() -> Self {
        NoCopySerde {
            str: SAMPLE_STR,
            bin: SAMPLE_BIN,
        }
    }
}

/// A reusable output buffer, so a benchmark measures encoding rather than
/// allocation.
///
/// Every library writes into the same kind of buffer with room to spare. That
/// matters: a tight buffer would push some encoders onto a slow fallback path
/// purely because of the sizing, and comparing those numbers would say more
/// about the harness than about the codec.
pub fn output_buffer(capacity: usize) -> Vec<u8> {
    Vec::with_capacity(capacity)
}

/// A dynamically shaped document, for comparing the `Value` paths.
pub fn value() -> CiboriumValue {
    ciborium::value::Value::Map(vec![
        (
            CiboriumValue::Text("name".into()),
            CiboriumValue::Text("zerocbor".into()),
        ),
        (
            CiboriumValue::Text("active".into()),
            CiboriumValue::Bool(true),
        ),
        (
            CiboriumValue::Text("version".into()),
            CiboriumValue::Integer(6.into()),
        ),
        (
            CiboriumValue::Text("ratio".into()),
            CiboriumValue::Float(1.25),
        ),
        (
            CiboriumValue::Text("tags".into()),
            CiboriumValue::Array(vec![
                CiboriumValue::Text("cbor".into()),
                CiboriumValue::Text("rust".into()),
                CiboriumValue::Text("zero-copy".into()),
                CiboriumValue::Text("no-std".into()),
            ]),
        ),
        (
            CiboriumValue::Text("authors".into()),
            CiboriumValue::Array(vec![
                CiboriumValue::Map(vec![
                    (
                        CiboriumValue::Text("name".into()),
                        CiboriumValue::Text("Alice".into()),
                    ),
                    (
                        CiboriumValue::Text("commits".into()),
                        CiboriumValue::Integer(127.into()),
                    ),
                    (
                        CiboriumValue::Text("active".into()),
                        CiboriumValue::Bool(true),
                    ),
                ]),
                CiboriumValue::Map(vec![
                    (
                        CiboriumValue::Text("name".into()),
                        CiboriumValue::Text("Bob".into()),
                    ),
                    (
                        CiboriumValue::Text("commits".into()),
                        CiboriumValue::Integer(63.into()),
                    ),
                    (
                        CiboriumValue::Text("active".into()),
                        CiboriumValue::Bool(false),
                    ),
                ]),
            ]),
        ),
        (
            CiboriumValue::Text("metrics".into()),
            CiboriumValue::Map(vec![
                (
                    CiboriumValue::Text("downloads".into()),
                    CiboriumValue::Array(vec![
                        CiboriumValue::Integer(0.into()),
                        CiboriumValue::Integer(1.into()),
                        CiboriumValue::Integer(127.into()),
                        CiboriumValue::Integer(128.into()),
                        CiboriumValue::Integer(255.into()),
                        CiboriumValue::Integer(256.into()),
                        CiboriumValue::Integer(65535.into()),
                        CiboriumValue::Integer(65536.into()),
                    ]),
                ),
                (
                    CiboriumValue::Text("latencies".into()),
                    CiboriumValue::Array(vec![
                        CiboriumValue::Float(0.25),
                        CiboriumValue::Float(1.5),
                        CiboriumValue::Float(12.75),
                        CiboriumValue::Float(100.125),
                    ]),
                ),
                (CiboriumValue::Text("nullable".into()), CiboriumValue::Null),
            ]),
        ),
    ])
}
