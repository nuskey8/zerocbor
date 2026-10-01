//! Wire-format and round-trip tests for `zerocbor`.
//!
//! The wire-format expectations come from RFC 8949 Appendix A ("Examples"),
//! which pins the exact bytes of every basic value. The float expectations are
//! the narrowed form, which is what Section 4.2.1 requires and what
//! `canonical.rs` covers in more depth; what is checked here is that the
//! Appendix vectors decode to the values they name.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Debug;

use zerocbor::{Error, Result, ToCbor, Value, Write};

/// Writes a homogeneous slice through [`ToCbor::write_slice`]. The concrete
/// writers are private, so reaching `write_slice` means implementing `ToCbor` —
/// the same route a user takes, so this covers the public contract.
struct WriteSlice<'a, T: ToCbor + Copy>(&'a [T]);

impl<T: ToCbor + Copy> ToCbor for WriteSlice<'_, T> {
    fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        T::write_slice(self.0, writer)
    }
}

/// Writes an `f32` in the half-float form. A plain `f32` never emits it because
/// it narrows, so reaching it needs a `Write`, and the concrete writers are
/// private — this is the one place that knows both.
struct AsF16(f32);

impl ToCbor for AsF16 {
    fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_f16(self.0)
    }
}

/// Encodes `value` and asserts the result matches `expected`.
#[track_caller]
fn assert_encodes<T: ToCbor + ?Sized + Debug>(value: &T, expected: &[u8]) {
    let encoded = zerocbor::to_cbor_vec(value).expect("serialization failed");
    assert_eq!(encoded, expected, "unexpected encoding of {value:?}");
}

/// Decodes `bytes` and asserts the result matches `expected`.
#[track_caller]
fn assert_decodes<T: zerocbor::FromCborOwned + Debug + PartialEq>(bytes: &[u8], expected: T) {
    let decoded: T = zerocbor::from_cbor(bytes).expect("deserialization failed");
    assert_eq!(decoded, expected, "unexpected decoding of {bytes:02x?}");
}

/// Encodes `value` as a CBOR half-float and returns the three bytes.
fn encode_f16(value: f32) -> [u8; 3] {
    let mut buf = [0u8; 3];
    let written = zerocbor::to_cbor(&AsF16(value), &mut buf).unwrap();
    assert_eq!(written, 3, "a half-float is always 3 bytes");
    buf
}

/// A byte string as hex, for a failure message.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether an error says the input ran out rather than that it was wrong. A
/// stream has no length to check against, so running out is how it fails, and
/// which error says so depends on the read that hit the end.
#[track_caller]
fn is_out_of_data(error: &Error) -> bool {
    matches!(error, Error::BufferTooSmall)
        || matches!(error, Error::IoError(e) if e.kind() == std::io::ErrorKind::UnexpectedEof)
}

/// The 16 bits of a half-float encoding, without the marker.
fn f16_bits(value: f32) -> u16 {
    let encoded = encode_f16(value);
    u16::from_be_bytes([encoded[1], encoded[2]])
}

mod appendix_a {
    use super::*;

    #[test]
    fn integers() {
        // The vectors from Appendix A, plus the boundaries of each argument
        // width: 23 fits in the head, 24 needs a byte, 65535 a `uint16`, and
        // 65536 a `uint32`.
        assert_encodes(&0u64, &[0x00]);
        assert_encodes(&1u64, &[0x01]);
        assert_encodes(&10u64, &[0x0a]);
        assert_encodes(&23u64, &[0x17]);
        assert_encodes(&24u64, &[0x18, 0x18]);
        assert_encodes(&25u64, &[0x18, 0x19]);
        assert_encodes(&100u64, &[0x18, 0x64]);
        assert_encodes(&1000u64, &[0x19, 0x03, 0xe8]);
        assert_encodes(&65535u64, &[0x19, 0xff, 0xff]);
        assert_encodes(&65536u64, &[0x1a, 0x00, 0x01, 0x00, 0x00]);
        assert_encodes(&1000000u64, &[0x1a, 0x00, 0x0f, 0x42, 0x40]);
        assert_encodes(
            &4294967296u64,
            &[0x1b, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
        );
        assert_encodes(
            &1000000000000u64,
            &[0x1b, 0x00, 0x00, 0x00, 0xe8, 0xd4, 0xa5, 0x10, 0x00],
        );
        assert_encodes(
            &u64::MAX,
            &[0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        );

        // Major type 1 stores `-1 - value`, so the payload is one more than the
        // magnitude.
        assert_encodes(&-1i64, &[0x20]);
        assert_encodes(&-10i64, &[0x29]);
        assert_encodes(&-24i64, &[0x37]);
        assert_encodes(&-25i64, &[0x38, 0x18]);
        assert_encodes(&-100i64, &[0x38, 0x63]);
        assert_encodes(&-256i64, &[0x38, 0xff]);
        assert_encodes(&-1000i64, &[0x39, 0x03, 0xe7]);
        assert_encodes(&-1000000i64, &[0x3a, 0x00, 0x0f, 0x42, 0x3f]);
        assert_encodes(
            &-1000000000000i64,
            &[0x3b, 0x00, 0x00, 0x00, 0xe8, 0xd4, 0xa5, 0x0f, 0xff],
        );
        assert_encodes(
            &i64::MIN,
            &[0x3b, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        );
    }

    #[test]
    fn floats_decode_to_the_values_they_name() {
        // The Appendix A vectors, checked as a decode: the encoder narrows to the
        // shortest width that holds the value, so the bytes it produces are not
        // the ones Appendix A shows for a value that fits more narrowly. What
        // matters is that each vector decodes to the value it names.
        assert_decodes(&[0xf9, 0x00, 0x00], 0.0f32);
        assert_decodes(&[0xf9, 0x3c, 0x00], 1.0f32);
        assert_decodes(&[0xf9, 0x3e, 0x00], 1.5f32);
        assert_decodes(&[0xfa, 0x47, 0xc3, 0x50, 0x00], 100000.0f32);
        assert_decodes(
            &[0xfb, 0x3f, 0xf1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a],
            1.1f64,
        );
        assert_decodes(
            &[0xfb, 0x41, 0x14, 0x58, 0x54, 0x00, 0x00, 0x00, 0x00],
            333333.0f64,
        );

        // And the narrowed form of a value is one of the vectors.
        assert_encodes(&1.5f32, &[0xf9, 0x3e, 0x00]);
        assert_encodes(&100000.0f32, &[0xfa, 0x47, 0xc3, 0x50, 0x00]);
    }

    #[test]
    fn strings() {
        assert_encodes(&"", &[0x60]);
        assert_encodes(&"a", &[0x61, 0x61]);
        assert_encodes(&"IETF", &[0x64, 0x49, 0x45, 0x54, 0x46]);
        assert_encodes(&"\"\\", &[0x62, 0x22, 0x5c]);
        // Each of these is a different number of bytes, so each takes a different
        // form of length argument.
        assert_encodes(&"ü", &[0x62, 0xc3, 0xbc]);
        assert_encodes(&"水", &[0x63, 0xe6, 0xb0, 0xb4]);
        assert_encodes(&"𐅑", &[0x64, 0xf0, 0x90, 0x85, 0x91]);
    }

    #[test]
    fn byte_strings() {
        // A byte string has to be asked for: a bare `&[u8]` serializes as an
        // array of integers, because the blanket `[T]` impl owns the type.
        #[derive(Debug)]
        struct ByteString<'a>(&'a [u8]);

        impl ToCbor for ByteString<'_> {
            fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
                writer.write_binary(self.0)
            }
        }

        assert_encodes(&ByteString(&b""[..]), &[0x40]);
        assert_encodes(
            &ByteString(&b"\x01\x02\x03\x04"[..]),
            &[0x44, 0x01, 0x02, 0x03, 0x04],
        );
    }

    #[test]
    fn containers() {
        assert_encodes::<[u32]>(&[], &[0x80]);
        assert_encodes(&[1u64, 2, 3], &[0x83, 0x01, 0x02, 0x03]);
        // A 25-element array needs the 8-bit length form (0x98).
        let twenty_fives = [1u32; 25];
        let expected = [&[0x98, 0x19][..], &[0x01; 25][..]].concat();
        assert_encodes(&twenty_fives, &expected);

        // A map is a head and then alternating keys and values.
        let map = BTreeMap::from([(1u64, 2u64), (3u64, 4u64)]);
        assert_encodes(&map, &[0xa2, 0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn an_integer_needs_one_more_bit_when_it_is_negative() {
        // Major type 1 stores `-1 - value`, so a negative integer needs one bit
        // more than a positive one of the same argument width. An 8-byte argument
        // with its top bit set therefore names a value below `i64::MIN`, and
        // subtracting in `i64` overflows: the answer came back positive, and a
        // document decoded to a different number than it held.
        //
        // The name is the one the format gives the magnitude, so `7fff...` is
        // `-2^63` and `8000...` is `-2^63 - 1`.
        for (argument, expected) in [
            (0xffff_ffff_ffff_ffffu64, -(1i128 << 64)), // the bottom, -2^64
            (0x8000_0000_0000_0000, -(1i128 << 63) - 1), // one past i64::MIN
            (0x7fff_ffff_ffff_ffff, -(1i128 << 63)),    // i64::MIN itself
            (0x0000_0000_0000_0000, -1),
        ] {
            let mut bytes = vec![0x3b];
            bytes.extend_from_slice(&argument.to_be_bytes());

            // The dynamic type holds the whole range, through both readers.
            for value in [
                zerocbor::from_cbor::<Value>(&bytes).expect("the wide value did not decode"),
                zerocbor::read_cbor::<_, Value>(std::io::Cursor::new(bytes.clone()))
                    .expect("the wide value did not decode from a stream"),
            ] {
                assert_eq!(
                    value,
                    Value::Integer(expected),
                    "for argument {argument:#018x}"
                );
                // And it goes back out as a value that decodes to the same thing.
                // The width is the shortest that holds the argument, so this is
                // not the nine bytes the input used for a small one.
                let encoded = zerocbor::to_cbor_vec(&value).unwrap();
                assert_eq!(
                    zerocbor::from_cbor::<Value>(&encoded).unwrap(),
                    value,
                    "for argument {argument:#018x}"
                );
                if argument > u32::MAX as u64 {
                    // Above that the 8-byte argument form is the only one there
                    // is, so these bytes are the only answer.
                    assert_eq!(encoded, bytes, "for argument {argument:#018x}");
                }
            }

            // A typed `i64` takes the two that fit and reports the rest, rather
            // than wrapping the way the subtraction used to.
            let typed = zerocbor::from_cbor::<i64>(&bytes);
            if let Ok(small) = i64::try_from(expected) {
                assert_eq!(typed.unwrap(), small, "for argument {argument:#018x}");
            } else {
                assert!(
                    matches!(typed.unwrap_err(), Error::IntegerOutOfRange),
                    "for argument {argument:#018x}"
                );
            }
        }

        // A positive argument reaches `u64::MAX`, which is one past `i64::MAX`
        // and so does not fit an `i64` but does fit the dynamic type.
        let mut bytes = vec![0x1b];
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(
            zerocbor::from_cbor::<Value>(&bytes).unwrap(),
            Value::Integer(u64::MAX as i128)
        );
        assert!(matches!(
            zerocbor::from_cbor::<u64>(&bytes).unwrap(),
            u64::MAX
        ));
        assert!(matches!(
            zerocbor::from_cbor::<i64>(&bytes).unwrap_err(),
            Error::IntegerOutOfRange
        ));

        // And an `i128` wider than the format's 64-bit argument is reported
        // rather than truncated into a different number.
        for out_of_range in [i128::from(u64::MAX) + 1, i128::MIN, -(1i128 << 64) - 1] {
            let err = zerocbor::to_cbor_vec(&Value::Integer(out_of_range)).unwrap_err();
            assert!(
                matches!(err, Error::IntegerOutOfRange),
                "{out_of_range} was truncated instead of reported"
            );
        }
    }

    #[test]
    fn the_two_byte_simple_value_form_carries_only_32_and_up() {
        // RFC 8949 Section 3.3 gives additional information 24 one meaning: a
        // simple value of 32 to 255 in the byte that follows. Everything below is
        // spoken for elsewhere — 20 through 23 are `false`, `true`, `null` and
        // `undefined`, which have their own single-byte forms, and 24 through 31
        // are the float widths, the break and the unassigned numbers.
        //
        // The writer only ever writes this form for 32 and up, so a value the
        // reader accepted below 32 could not be written back out. Accepting
        // `f8 14` would also let one document mean two things: `false` written
        // as `f4` reads back as `Value::Bool(false)`, but the same number
        // written as `f8 14` reads back as `Value::Simple(20)`.
        for value in 0u8..=31 {
            // Bare, and nested in an array and in a map, so the rejection is the
            // value's and not the container's.
            for bytes in [
                vec![0xf8, value],
                vec![0x81, 0xf8, value],
                vec![0xa1, 0x00, 0xf8, value],
            ] {
                let err = zerocbor::from_cbor::<Value>(&bytes).unwrap_err();
                assert!(
                    matches!(err, Error::InvalidSimpleValue(n) if n == value),
                    "f8 {value:02x} was accepted: {err:?}"
                );
            }
        }

        for value in [32u8, 100, 254, 255] {
            let encoded = [0xf8, value];
            let decoded = zerocbor::from_cbor::<Value>(&encoded).unwrap();
            assert_eq!(decoded, Value::Simple(value), "f8 {value:02x}");
            assert_eq!(zerocbor::to_cbor_vec(&decoded).unwrap(), encoded);
        }
    }
    #[test]
    fn simple_values_and_tags() {
        assert_encodes(&false, &[0xf4]);
        assert_encodes(&true, &[0xf5]);
        assert_encodes(&(), &[0xf6]);
        assert_encodes(&None::<u64>, &[0xf6]);

        // 0("2013-03-21T20:04:00Z") — the date/time tag from Appendix A.
        let value = Value::Tag(
            0,
            Box::new(Value::Text(Cow::Borrowed("2013-03-21T20:04:00Z"))),
        );
        assert_encodes(
            &value,
            &[
                0xc0, 0x74, 0x32, 0x30, 0x31, 0x33, 0x2d, 0x30, 0x33, 0x2d, 0x32, 0x31, 0x54, 0x32,
                0x30, 0x3a, 0x30, 0x34, 0x3a, 0x30, 0x30, 0x5a,
            ],
        );
    }
}

mod round_trip {
    use super::*;

    #[test]
    fn scalars_round_trip() {
        for value in [
            0i64,
            1,
            -1,
            23,
            24,
            -24,
            -25,
            255,
            256,
            u16::MAX as i64,
            i32::MAX as i64,
            i64::MIN,
            i64::MAX,
            i32::MIN as i64,
        ] {
            assert_decodes(&zerocbor::to_cbor_vec(&value).unwrap(), value);
        }
        for value in ["", "a", "hello", "ü", "水", "𐅑"] {
            assert_decodes(&zerocbor::to_cbor_vec(&value).unwrap(), value.to_string());
        }
        // A `NaN` never compares equal, so it is checked by kind.
        let decoded: f64 = zerocbor::from_cbor(&zerocbor::to_cbor_vec(&f64::NAN).unwrap()).unwrap();
        assert!(decoded.is_nan());
    }

    #[test]
    fn collections_round_trip() {
        // A tuple, a byte string, an option in both states, and a nest of them.
        assert_decodes(
            &zerocbor::to_cbor_vec(&(1u8, "two".to_string(), 3.5f64, vec![true, false])).unwrap(),
            (1u8, "two".to_string(), 3.5f64, vec![true, false]),
        );
        assert_decodes(
            &zerocbor::to_cbor_vec(&&[0u8, 1, 2, 255][..]).unwrap(),
            vec![0u8, 1, 2, 255],
        );
        assert_decodes(&zerocbor::to_cbor_vec(&Some(7u32)).unwrap(), Some(7u32));
        assert_decodes(&zerocbor::to_cbor_vec(&None::<u32>).unwrap(), None::<u32>);
        let nested: Vec<Option<Vec<u64>>> = vec![Some(vec![1, 2, 3]), None, Some(Vec::new())];
        assert_decodes(&zerocbor::to_cbor_vec(&nested).unwrap(), nested);
    }

    #[test]
    fn a_value_round_trips() {
        let original = Value::Map(BTreeMap::from([
            (Value::Text(Cow::Borrowed("a")), Value::Integer(1)),
            (
                Value::Text(Cow::Borrowed("b")),
                Value::Array(vec![Value::Bool(true)]),
            ),
        ]));
        let encoded = zerocbor::to_cbor_vec(&original).unwrap();
        let decoded: Value<'_> = zerocbor::from_cbor(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn a_borrowed_str_is_read_without_copying() {
        // The decoded `&str` has the lifetime of the input, which is the
        // property: it could only be a borrow.
        let encoded = zerocbor::to_cbor_vec("borrowed").unwrap();
        let decoded: &str = zerocbor::from_cbor(&encoded).unwrap();
        assert_eq!(decoded, "borrowed");
        assert!(core::ptr::eq(decoded.as_ptr(), encoded[1..].as_ptr()));
    }

    #[test]
    fn decoding_stops_at_the_end_of_the_value() {
        // Trailing bytes must be left for the next value rather than consumed.
        let data = [0x01, 0x02, 0x03];
        let first: u64 = zerocbor::from_cbor(&data).unwrap();
        let rest: u64 = zerocbor::from_cbor(&data[1..]).unwrap();
        assert_eq!((first, rest), (1, 2));
    }
}

mod errors {
    use super::*;

    #[test]
    fn malformed_input_is_reported() {
        // Every way a value can be wrong, and the error each one is. The
        // indefinite-length forms are covered by `indefinite_length.rs`.
        for (label, result) in [
            (
                "a truncated argument",
                zerocbor::from_cbor::<u64>(&[0x19, 0x03]).map(|_| ()),
            ),
            (
                "a byte string where an integer belongs",
                zerocbor::from_cbor::<u64>(&[0x41, 0x00]).map(|_| ()),
            ),
            (
                "an array of the wrong length",
                zerocbor::from_cbor::<[u8; 3]>(&[0x82, 0x01, 0x02]).map(|_| ()),
            ),
            (
                "invalid UTF-8 in a text string",
                zerocbor::from_cbor::<String>(&[0x62, 0xff, 0xfe]).map(|_| ()),
            ),
            (
                "a break outside any container",
                zerocbor::from_cbor::<u8>(&[0xff]).map(|_| ()),
            ),
            (
                "a length of 31 on a scalar",
                zerocbor::from_cbor::<u8>(&[0x1f]).map(|_| ()),
            ),
            (
                "an integer too wide for the target",
                zerocbor::from_cbor::<u8>(&[0x19, 0x01, 0x00]).map(|_| ()),
            ),
        ] {
            let err = result.expect_err(label);
            assert!(
                !format!("{err}").is_empty(),
                "{label}: the error says nothing"
            );
        }
    }

    #[test]
    fn a_length_claim_is_not_believed() {
        // A nine-byte header claiming four billion elements must fail on the
        // missing data, not on a four-billion-element reservation.
        let err = zerocbor::from_cbor::<Vec<u64>>(&[
            0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ])
        .unwrap_err();
        assert!(matches!(err, Error::BufferTooSmall), "got {err:?}");
    }

    #[test]
    fn a_stream_does_not_believe_a_length_claim_either() {
        // A slice reader checks a count against the bytes that remain, so it
        // knows a claim is impossible before growing anything. A stream has no
        // such check available, which makes it the place a length claim from the
        // wire could turn a ten-byte document into a large allocation. Each of
        // these claims 150 gigabytes, or one billion elements, and every input
        // is shorter than its claim.
        //
        // A slice is checked too, so a fix that only covered one of the two
        // readers would show up as one of these failing.
        const HUGE: usize = 0x2300_00ff00;
        for bytes in [
            // [_ with 2^40 elements declared.
            vec![0x9b, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00],
            // (h'...') with `HUGE` bytes declared.
            {
                let mut v = vec![0x5b];
                v.extend_from_slice(&(HUGE as u64).to_be_bytes());
                v
            },
            // (_ with a chunk of `HUGE` bytes declared, which is the path that
            // reached the allocation: the chunk is inside an indefinite-length
            // text string, so the joined result is what would be allocated.
            // 0x7b is a text chunk with an 8-byte length argument, which is the
            // shape the chunk has to have to reach that code at all.
            {
                let mut v = vec![0x7f, 0x7b];
                v.extend_from_slice(&(HUGE as u64).to_be_bytes());
                v.push(0xff);
                v
            },
        ] {
            let label = hex(&bytes);
            for reader in [0, 1] {
                // The slice reader and the stream reader are the two paths, and
                // the same bytes go to each.
                let result = if reader == 0 {
                    zerocbor::from_cbor::<Value>(&bytes)
                } else {
                    zerocbor::read_cbor::<_, Value>(std::io::Cursor::new(bytes.clone()))
                };
                let err = result.expect_err("a length claim was believed");
                assert!(is_out_of_data(&err), "{label}: got {err:?}");
            }
        }
    }

    #[test]
    fn a_buffer_that_is_exactly_the_right_size_works_and_one_byte_less_does_not() {
        let encoded = zerocbor::to_cbor_vec(&42u64).unwrap();
        let mut exact = vec![0u8; encoded.len()];
        assert_eq!(
            zerocbor::to_cbor(&42u64, &mut exact).unwrap(),
            encoded.len()
        );
        assert_eq!(exact, encoded);

        let mut short = vec![0u8; encoded.len() - 1];
        let err = zerocbor::to_cbor(&42u64, &mut short).unwrap_err();
        assert!(matches!(err, Error::BufferTooSmall), "got {err:?}");
    }

    #[test]
    fn nesting_is_bounded_and_the_bound_is_not_in_the_way() {
        // Decoding a container recurses, so a deeply nested input has to stop
        // somewhere rather than exhaust the stack.
        let at_limit = vec![0x81u8; zerocbor::MAX_DEPTH - 1];
        let mut within = at_limit.clone();
        within.push(0x00);
        assert!(
            zerocbor::from_cbor::<Value>(&within).is_ok(),
            "the limit is too tight"
        );

        let over = vec![0x81u8; zerocbor::MAX_DEPTH + 1];
        let mut past = over.clone();
        past.push(0x00);
        let err = zerocbor::from_cbor::<Value>(&past).unwrap_err();
        assert!(matches!(err, Error::DepthLimitExceeded), "got {err:?}");
    }
}

mod bulk_slices {
    use super::*;

    #[test]
    fn a_bulk_write_fits_the_real_encoding_and_reports_a_short_buffer() {
        // Exactly enough room for the real encoding: the bulk path must not
        // demand the 9-byte-per-element worst case here. The wrapper reports no
        // size hint, so `to_cbor` takes the checked path and the encoder has to
        // fit the real bytes.
        let values = [1u64, 2, 3];
        let expected: Vec<u8> = values
            .iter()
            .flat_map(|v| zerocbor::to_cbor_vec(v).unwrap())
            .collect();

        let mut exact = vec![0u8; expected.len()];
        let written = zerocbor::to_cbor(&WriteSlice(&values), &mut exact).unwrap();
        assert_eq!(&exact[..written], &expected[..]);

        let mut short = vec![0u8; expected.len() - 1];
        let err = zerocbor::to_cbor(&WriteSlice(&values), &mut short).unwrap_err();
        assert!(matches!(err, Error::BufferTooSmall), "got {err:?}");
    }
}

mod half_precision {
    use super::*;

    /// `binary16` values that are exactly representable, so they must survive a
    /// round trip unchanged. The two subnormals are the cases a naive
    /// conversion gets wrong by flushing to zero.
    const INTERESTING: [f32; 11] = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        65504.0, // largest finite binary16
        -65504.0,
        6.1035156e-5, // largest subnormal, 2^-14
        5.9604645e-8, // smallest subnormal, 2^-24
        f32::INFINITY,
        f32::NEG_INFINITY,
    ];

    #[test]
    fn every_representable_value_survives_a_round_trip() {
        for value in INTERESTING {
            let encoded = encode_f16(value);
            assert_eq!(encoded[0], 0xf9, "expected a half-float for {value}");
            // Widening is what both widths have to accept, since a value may be
            // written narrower than the type reading it.
            let as_f32: f32 = zerocbor::from_cbor(&encoded).unwrap();
            let as_f64: f64 = zerocbor::from_cbor(&encoded).unwrap();
            assert_eq!(as_f32, value, "{value} did not survive as an f32");
            assert_eq!(as_f64, value as f64, "{value} did not survive as an f64");
        }
    }

    #[test]
    fn the_full_binary16_space_round_trips() {
        // All 65536 bit patterns, which is what makes the boundaries above a
        // sample rather than the argument. A binary16 is exactly representable
        // in an f32, so this is the identity everywhere except a NaN, which only
        // has to stay a NaN.
        for bits in 0u16..=u16::MAX {
            let encoded = [0xf9, (bits >> 8) as u8, bits as u8];
            let decoded: f32 = zerocbor::from_cbor(&encoded)
                .unwrap_or_else(|e| panic!("failed to decode {bits:#06x}: {e}"));
            let back = encode_f16(decoded);
            if decoded.is_nan() {
                assert_eq!(back[0], 0xf9, "a NaN lost its half-float marker");
                continue;
            }
            assert_eq!(
                u16::from_be_bytes([back[1], back[2]]),
                bits,
                "{bits:#06x} did not survive a round trip"
            );
        }
    }

    #[test]
    fn narrowing_rounds_to_nearest_even_and_overflows_to_infinity() {
        // Ties to even, which is what the significand's low bit decides: a tie
        // rounds to the even neighbour, down to zero and up to 0x0002.
        for (value, expected_bits) in [
            (2.9802322e-8, 0x0000u16), // exactly half of the smallest
            (4.4703484e-8, 0x0001),    // 1.5 * 2^-25, rounds up
            (5.9604645e-8, 0x0001),    // exact
            (8.940697e-8, 0x0002),     // 1.5 * 2^-24, ties up
            (1.0, 0x3c00),
            (1.5, 0x3e00),
            (65504.0, 0x7bff), // the largest finite half
            // 65520 is the midpoint between 65504 and the overflowing value, so
            // ties to even sends it to infinity.
            (65520.0, 0x7c00),
            (f32::INFINITY, 0x7c00),
            (f32::NEG_INFINITY, 0xfc00),
        ] {
            assert_eq!(
                f16_bits(value),
                expected_bits,
                "wrong narrowing for {value}"
            );
        }
        // Just below the midpoint stays finite.
        assert_eq!(f16_bits(65519.0), 0x7bff, "65519 must stay finite");

        // A `NaN` has no value to compare, so it is checked by kind.
        let decoded: f32 = zerocbor::from_cbor(&encode_f16(f32::NAN)).unwrap();
        assert!(decoded.is_nan());
    }
}

mod io {
    #[test]
    fn a_value_round_trips_through_a_stream() {
        let value = vec![(1u64, "one".to_string()), (2, "two".to_string())];
        let mut buf = Vec::new();
        zerocbor::write_cbor(&mut buf, &value).unwrap();
        // And it is the same bytes a slice writer would produce.
        assert_eq!(buf, zerocbor::to_cbor_vec(&value).unwrap());

        let decoded: Vec<(u64, String)> = zerocbor::read_cbor(std::io::Cursor::new(buf)).unwrap();
        assert_eq!(decoded, value);
    }
}
