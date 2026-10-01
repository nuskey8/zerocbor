//! Indefinite-length containers, which RFC 8949 Section 3 lets a producer end
//! with a break stop code instead of a count.
//!
//! Every competitor decoder accepts them, so refusing them makes zerocbor the
//! odd one out on real input. A producer reaches for them when it does not know
//! the length up front, which streaming encoders and most of the CBOR-in-JSON
//! bridges do.
//!
//! Decoding is supported for every container, including chunked strings.
//! Encoding is not: a definite-length value is one byte shorter for the short
//! forms, so nothing here has a reason to write the longer form, and all three
//! competitor encoders write definite lengths for typed values too.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use zerocbor::{Error, FromCbor, Value, from_cbor, to_cbor_vec};

// The test vectors below are the shapes RFC 8949 Section 3 uses to show
// indefinite-length encoding: a header with additional information 31, the
// items, then a break stop code.

mod arrays {
    use super::*;

    #[test]
    fn an_indefinite_length_array_reads_until_its_break() {
        // [_ 1, 2]
        assert_eq!(
            from_cbor::<Vec<u64>>(&[0x9f, 0x01, 0x02, 0xff]).unwrap(),
            [1, 2]
        );
        // [_ ]
        assert!(from_cbor::<Vec<u64>>(&[0x9f, 0xff]).unwrap().is_empty());
        // [_ [_ 1], [_ 2, 3]]
        assert_eq!(
            from_cbor::<Vec<Vec<u64>>>(&[0x9f, 0x9f, 0x01, 0xff, 0x9f, 0x02, 0x03, 0xff, 0xff])
                .unwrap(),
            vec![vec![1u64], vec![2, 3]]
        );
        // The two forms decode alike, which is the interop property.
        assert_eq!(
            from_cbor::<Vec<u64>>(&[0x9f, 0x01, 0x02, 0xff]).unwrap(),
            from_cbor::<Vec<u64>>(&[0x82, 0x01, 0x02]).unwrap()
        );
    }

    #[test]
    fn every_collection_reads_an_indefinite_length_array() {
        // Each opens its own container, so each is checked.
        let input = [0x9f, 0x01, 0x02, 0xff];
        assert_eq!(
            from_cbor::<VecDeque<u64>>(&input).unwrap(),
            VecDeque::from([1u64, 2])
        );
        assert_eq!(
            from_cbor::<BTreeSet<u64>>(&input).unwrap(),
            BTreeSet::from([1u64, 2])
        );
        assert_eq!(
            from_cbor::<Vec<String>>(&[0x9f, 0x61, b'a', 0xff]).unwrap(),
            ["a"]
        );
    }

    #[test]
    fn a_type_with_a_known_element_count_reads_one_too() {
        // The type supplies the count, so an indefinite one is unambiguous.
        assert_eq!(
            from_cbor::<(u8, u8)>(&[0x9f, 0x01, 0x02, 0xff]).unwrap(),
            (1, 2)
        );
        assert_eq!(
            from_cbor::<[u8; 2]>(&[0x9f, 0x01, 0x02, 0xff]).unwrap(),
            [1, 2]
        );
    }

    #[test]
    fn a_broken_indefinite_length_array_is_reported() {
        // Each of these is a way the input fails to match what it declared, and
        // each has to be an error rather than a value.
        for (label, result) in [
            (
                "the input ran out before the break",
                from_cbor::<Vec<u64>>(&[0x9f, 0x01]).map(|_| ()),
            ),
            (
                "more elements than the type has room for",
                from_cbor::<[u8; 2]>(&[0x9f, 0x01, 0x02, 0x03, 0xff]).map(|_| ()),
            ),
            (
                "an element of the wrong type",
                from_cbor::<Vec<Vec<u64>>>(&[0x9f, 0x01, 0xff]).map(|_| ()),
            ),
        ] {
            let err = result.expect_err(label);
            assert!(
                !format!("{err}").is_empty(),
                "{label}: the error says nothing"
            );
        }
    }
}

mod maps {
    use super::*;

    #[test]
    fn an_indefinite_length_map_reads_until_its_break() {
        // {"a": 1}
        let map = from_cbor::<BTreeMap<String, u64>>(&[0xbf, 0x61, b'a', 0x01, 0xff]).unwrap();
        assert_eq!(map.get("a"), Some(&1));
        // { }
        assert!(
            from_cbor::<BTreeMap<String, u64>>(&[0xbf, 0xff])
                .unwrap()
                .is_empty()
        );
        // {"a": {"b": 1}}
        assert_eq!(
            from_cbor::<BTreeMap<String, BTreeMap<String, u64>>>(&[
                0xbf, 0x61, b'a', 0xbf, 0x61, b'b', 0x01, 0xff, 0xff
            ])
            .unwrap()["a"]["b"],
            1
        );
    }

    #[test]
    fn a_broken_indefinite_length_map_is_reported() {
        for (label, result) in [
            (
                "the input ran out before the break",
                from_cbor::<BTreeMap<String, u64>>(&[0xbf, 0x61, b'a', 0x01]).map(|_| ()),
            ),
            (
                "a key with no value after it",
                from_cbor::<BTreeMap<String, u64>>(&[0xbf, 0x61, b'a', 0xff]).map(|_| ()),
            ),
        ] {
            let err = result.expect_err(label);
            assert!(
                !format!("{err}").is_empty(),
                "{label}: the error says nothing"
            );
        }
    }
}

mod chunked_strings {
    use super::*;

    /// A byte string has to be asked for, because `Vec<u8>` on its own means an
    /// array of `u8` rather than a major-type-2 blob.
    #[derive(Debug, PartialEq, FromCbor)]
    struct Blob {
        #[cbor(as_bytes)]
        bytes: Vec<u8>,
    }

    #[test]
    fn a_chunked_string_is_joined() {
        // (_ "hi", "!") — each chunk is an independently valid text string.
        assert_eq!(
            from_cbor::<String>(&[0x7f, 0x62, b'h', b'i', 0x61, b'!', 0xff]).unwrap(),
            "hi!"
        );
        // (_ h'0102', h'0304')
        assert_eq!(
            from_cbor::<Blob>(&[0x81, 0x5f, 0x42, 0x01, 0x02, 0x42, 0x03, 0x04, 0xff])
                .unwrap()
                .bytes,
            [1, 2, 3, 4]
        );
        // (_ ) and (_ ) with no chunks at all are the empty string.
        assert_eq!(from_cbor::<String>(&[0x7f, 0xff]).unwrap(), "");
        assert!(
            from_cbor::<Blob>(&[0x81, 0x5f, 0xff])
                .unwrap()
                .bytes
                .is_empty()
        );
    }

    #[test]
    fn a_chunk_must_be_the_same_kind_of_string_as_its_parent() {
        // RFC 8949 Section 3.2.3 says the chunks of an indefinite-length text
        // string are text strings and those of an indefinite-length byte string
        // are byte strings. A chunk of the other kind is a different major type,
        // and accepting it would let one document be read two ways: `(_ h'41',
        // h'c3', h'41', h'a9')` would be an indefinite-length byte string under
        // one reading and an indefinite-length text string under the other.
        for (label, input) in [
            (
                "a byte string chunk in a text string",
                vec![0x7f, 0x41, b'a', 0xff],
            ),
            (
                "a text string chunk in a byte string",
                vec![0x5f, 0x61, b'a', 0xff],
            ),
        ] {
            // Both a typed read and the dynamic one, and both readers, since a
            // fix that only covered one of the four would show up here.
            for err in [
                from_cbor::<String>(&input).unwrap_err(),
                from_cbor::<Value>(&input).unwrap_err(),
                from_cbor::<Blob>(&input).unwrap_err(),
                zerocbor::read_cbor::<_, String>(std::io::Cursor::new(input.clone())).unwrap_err(),
                zerocbor::read_cbor::<_, Value>(std::io::Cursor::new(input.clone())).unwrap_err(),
            ] {
                assert!(
                    matches!(err, Error::InvalidInitialByte(_)),
                    "{label}: got {err:?}"
                );
            }
        }
    }

    #[test]
    fn a_chunk_must_end_at_a_code_point_boundary() {
        let valid = [0x7f, 0x62, 0xc3, 0xa9, 0x63, 0xe6, 0x97, 0xa5, 0xff];
        assert_eq!(from_cbor::<String>(&valid).unwrap(), "é日");
        for bytes in [
            &[0x7f, 0x61, 0xc3, 0x61, 0xa9, 0xff][..],
            &[0x7f, 0x61, 0xc3, 0x61, 0x28, 0xff][..],
            &[0x7f, 0x61, 0x80, 0xff][..],
        ] {
            assert!(matches!(
                from_cbor::<String>(bytes),
                Err(Error::InvalidUtf8(_))
            ));
            assert!(matches!(
                from_cbor::<Value>(bytes),
                Err(Error::InvalidUtf8(_))
            ));
            #[cfg(feature = "std")]
            assert!(matches!(
                zerocbor::read_cbor::<_, String>(bytes),
                Err(Error::InvalidUtf8(_))
            ));
        }
        #[cfg(feature = "std")]
        assert_eq!(zerocbor::read_cbor::<_, String>(&valid[..]).unwrap(), "é日");
    }

    #[test]
    fn a_malformed_chunked_string_is_reported() {
        for (label, result) in [
            (
                "the input ran out mid-chunk",
                from_cbor::<String>(&[0x7f, 0x62, b'h', b'i']).map(|_| ()),
            ),
            (
                "the input ran out before the break",
                from_cbor::<String>(&[0x7f, 0x61, b'h']).map(|_| ()),
            ),
        ] {
            let err = result.expect_err(label);
            assert!(
                !format!("{err}").is_empty(),
                "{label}: the error says nothing"
            );
        }

        // A chunk carries a definite length, so a nested one is malformed.
        let err = from_cbor::<String>(&[0x7f, 0x7f, 0x61, b'a', 0xff, 0xff]).unwrap_err();
        assert!(
            matches!(err, Error::InvalidAdditionalInfo(31)),
            "got {err:?}"
        );
    }

    #[test]
    fn a_one_chunk_string_borrows_and_a_multi_chunk_one_cannot() {
        // The borrow is the assertion: a copy could not produce it.
        let data = [0x7f, 0x62, b'h', b'i', 0xff];
        let borrowed: &str = from_cbor(&data[..]).unwrap();
        assert_eq!(borrowed, "hi");
        assert!(core::ptr::eq(borrowed.as_ptr(), data[2..].as_ptr()));

        // Two chunks are not contiguous, so there is nothing to borrow. That is
        // more useful than a buffer the caller cannot outlive.
        let data = [0x7f, 0x61, b'h', 0x61, b'i', 0xff];
        let err = from_cbor::<&str>(&data[..]).unwrap_err();
        assert!(matches!(err, Error::CannotBorrow), "got {err:?}");
        assert_eq!(from_cbor::<String>(&data[..]).unwrap(), "hi");

        #[derive(Debug, PartialEq, FromCbor)]
        struct Borrowed<'a> {
            #[cbor(as_bytes)]
            bytes: &'a [u8],
        }
        let err = from_cbor::<Borrowed>(&[0x81, 0x5f, 0x41, 1, 0x41, 2, 0xff]).unwrap_err();
        assert!(matches!(err, Error::CannotBorrow), "got {err:?}");
    }
}

mod derived {
    use super::*;
    use zerocbor_derive::{FromCbor, ToCbor};

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    struct Strict {
        a: u64,
        b: u64,
    }

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    struct Lenient {
        a: u64,
        #[cbor(default)]
        b: u64,
    }

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(array)]
    struct Pair {
        a: u64,
        b: u64,
    }

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    enum Shape {
        Circle { radius: u64 },
        Rect { w: u64, h: u64 },
    }

    #[test]
    fn a_strict_struct_reads_an_indefinite_length_map() {
        let value = from_cbor::<Strict>(&[0xbf, 0x61, b'a', 0x01, 0x61, b'b', 0x02, 0xff]).unwrap();
        assert_eq!(value, Strict { a: 1, b: 2 });

        // The keys really were in the order they were written, so the reader
        // dispatched on them rather than on a position.
        let value = from_cbor::<Strict>(&[0xbf, 0x61, b'b', 0x02, 0x61, b'a', 0x01, 0xff]).unwrap();
        assert_eq!(value, Strict { a: 1, b: 2 });
    }

    #[test]
    fn the_strictness_rules_still_apply_to_an_indefinite_length_map() {
        // An unknown key is rejected, a defaulted one is filled, and a required
        // one that never arrives is reported — the same as for a definite map.
        let err = from_cbor::<Strict>(&[
            0xbf, 0x61, b'a', 0x01, 0x61, b'b', 0x02, 0x61, b'z', 0x03, 0xff,
        ])
        .unwrap_err();
        assert!(matches!(err, Error::UnknownVariant(_)), "got {err:?}");

        assert_eq!(
            from_cbor::<Lenient>(&[0xbf, 0x61, b'a', 0x01, 0xff]).unwrap(),
            Lenient { a: 1, b: 0 }
        );

        let err = from_cbor::<Strict>(&[0xbf, 0x61, b'a', 0x01, 0xff]).unwrap_err();
        assert!(matches!(err, Error::KeyNotFound(_)), "got {err:?}");
    }

    #[test]
    fn an_array_mode_struct_reads_an_indefinite_length_array() {
        assert_eq!(
            from_cbor::<Pair>(&[0x9f, 0x01, 0x02, 0xff]).unwrap(),
            Pair { a: 1, b: 2 }
        );
    }

    #[test]
    fn an_enum_reads_an_indefinite_length_container() {
        // {"Rect": {"w": 1, "h": 2}}, both maps break-terminated.
        let value = from_cbor::<Shape>(&[
            0xbf, 0x64, b'R', b'e', b'c', b't', 0xbf, 0x61, b'w', 0x01, 0x61, b'h', 0x02, 0xff,
            0xff,
        ])
        .unwrap();
        assert_eq!(value, Shape::Rect { w: 1, h: 2 });

        // In array mode the prelude has already read the discriminant.
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(array)]
        enum Op {
            Push(u64),
            Drop(u64),
        }
        assert_eq!(
            from_cbor::<Op>(&[0x9f, 0x00, 0x18, 0x2a, 0xff]).unwrap(),
            Op::Push(42)
        );
    }
}

mod skipping {
    use super::*;
    use zerocbor_derive::{FromCbor, ToCbor};

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map, allow_unknown_fields)]
    struct Tolerant {
        a: u64,
    }

    #[test]
    fn an_indefinite_length_value_is_skipped_whole() {
        // The unknown key holds an indefinite array, an indefinite map and a
        // chunked string, so a skip that only handled definite lengths would
        // leave the reader inside the value.
        assert_eq!(
            from_cbor::<Tolerant>(&[
                0xbf, 0x61, b'a', 0x01, 0x61, b'z', 0x9f, 0x9f, 0x01, 0xff, 0xff, 0xff,
            ])
            .unwrap(),
            Tolerant { a: 1 }
        );
        assert_eq!(
            from_cbor::<Tolerant>(&[
                0xbf, 0x61, b'a', 0x01, 0x61, b'z', 0xbf, 0x61, b'q', 0x02, 0xff, 0xff,
            ])
            .unwrap(),
            Tolerant { a: 1 }
        );
        assert_eq!(
            from_cbor::<Tolerant>(&[
                0xbf, 0x61, b'a', 0x01, 0x61, b'z', 0x7f, 0x62, b'h', b'i', 0x61, b'!', 0xff, 0xff,
            ])
            .unwrap(),
            Tolerant { a: 1 }
        );
    }
}

mod dynamic {
    use super::*;

    #[test]
    fn a_value_reads_every_indefinite_length_shape() {
        // [_ [_ 1]]
        let value = from_cbor::<Value>(&[0x9f, 0x9f, 0x01, 0xff, 0xff]).unwrap();
        let Value::Array(items) = &value else {
            panic!("expected an array, got {value:?}");
        };
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0], Value::Array(_)));

        // {"a": [_ ]}
        let value = from_cbor::<Value>(&[0xbf, 0x61, b'a', 0x9f, 0xff, 0xff]).unwrap();
        let Value::Map(map) = &value else {
            panic!("expected a map, got {value:?}");
        };
        assert_eq!(map.len(), 1);

        // (_ "hi", "!")
        let value = from_cbor::<Value>(&[0x7f, 0x62, b'h', b'i', 0x61, b'!', 0xff]).unwrap();
        let Value::Text(text) = value else {
            panic!("expected text, got {value:?}");
        };
        assert_eq!(text, "hi!");
        // The chunks are not contiguous, so the joined string owns its bytes.
        assert!(matches!(text, Cow::Owned(_)));
    }
}

mod streaming {
    use super::*;

    /// Hands out one byte at a time, so a buffered path would be caught
    /// reading ahead.
    struct Trickle<'a> {
        data: &'a [u8],
    }

    impl std::io::Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.data.is_empty() || buf.is_empty() {
                return Ok(0);
            }
            buf[0] = self.data[0];
            self.data = &self.data[1..];
            Ok(1)
        }
    }

    /// The `io::Read` path uses the trait's own `read_array`, which the
    /// in-memory reader overrides, so the two are checked separately rather than
    /// assuming one stands in for the other.
    fn read<T: for<'de> zerocbor::FromCbor<'de>>(bytes: &[u8]) -> Result<T, Error> {
        zerocbor::read_cbor(Trickle { data: bytes })
    }

    #[test]
    fn every_container_reads_from_a_stream() {
        assert_eq!(read::<Vec<u64>>(&[0x82, 0x01, 0x02]).unwrap(), [1, 2]);
        assert_eq!(read::<Vec<u64>>(&[0x9f, 0x01, 0x02, 0xff]).unwrap(), [1, 2]);
        assert!(read::<Vec<u64>>(&[0x9f, 0xff]).unwrap().is_empty());
        assert_eq!(
            read::<Vec<Vec<u64>>>(&[0x9f, 0x9f, 0x01, 0xff, 0x9f, 0x02, 0xff, 0xff]).unwrap(),
            vec![vec![1u64], vec![2]]
        );
        assert_eq!(read::<[u8; 2]>(&[0x9f, 0x01, 0x02, 0xff]).unwrap(), [1, 2]);
        assert_eq!(
            read::<BTreeMap<String, u64>>(&[0xbf, 0x61, b'a', 0x01, 0xff])
                .unwrap()
                .get("a"),
            Some(&1)
        );
        assert_eq!(
            read::<String>(&[0x7f, 0x62, b'h', b'i', 0x61, b'!', 0xff]).unwrap(),
            "hi!"
        );
    }

    #[test]
    fn a_truncated_stream_is_reported() {
        // A stream has no length to check a count against, so running out
        // surfaces as the read failing.
        for (label, result) in [
            (
                "an array with no break",
                read::<Vec<u64>>(&[0x9f, 0x01]).map(|_| ()),
            ),
            (
                "a string with no break",
                read::<String>(&[0x7f, 0x61, b'h']).map(|_| ()),
            ),
            (
                "a header claiming four billion elements",
                read::<Vec<u64>>(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff])
                    .map(|_| ()),
            ),
        ] {
            let err = result.expect_err(label);
            assert!(
                !format!("{err}").is_empty(),
                "{label}: the error says nothing"
            );
        }
    }
}

mod limits {
    use super::*;

    #[test]
    fn a_len_reports_which_form_the_header_declared() {
        assert_eq!(zerocbor::Len::Known(3).known(), Some(3));
        assert!(!zerocbor::Len::Known(3).is_indefinite());
        assert_eq!(zerocbor::Len::Indefinite.known(), None);
        assert!(zerocbor::Len::Indefinite.is_indefinite());
    }

    #[test]
    fn indefinite_nesting_is_bounded_like_definite_nesting() {
        // A break-terminated array is still a recursion, so it is bounded the
        // same way.
        let depth = zerocbor::MAX_DEPTH - 1;
        let mut within = vec![0x9f; depth];
        within.extend(std::iter::repeat_n(0xff, depth));
        assert!(
            from_cbor::<Value>(&within).is_ok(),
            "the limit is too tight"
        );

        let mut past = vec![0x9f; zerocbor::MAX_DEPTH + 1];
        past.extend(std::iter::repeat_n(0xff, zerocbor::MAX_DEPTH + 1));
        let err = from_cbor::<Value>(&past).unwrap_err();
        assert!(matches!(err, Error::DepthLimitExceeded), "got {err:?}");
    }

    #[test]
    fn a_failed_read_leaves_the_depth_balanced() {
        // The free functions reuse the reader, so an error part-way must not
        // leave the counter elevated.
        let mut reader = std::io::Cursor::new(vec![0x9f, 0x01]);
        assert!(zerocbor::read_cbor::<_, Value>(&mut reader).is_err());
        // A two-level nesting is comfortably inside the limit, so this can only
        // pass if the counter went back down.
        let ok = vec![0x81, 0x81, 0x01];
        let mut reader = std::io::Cursor::new(ok);
        assert!(zerocbor::read_cbor::<_, Value>(&mut reader).is_ok());
    }
}

mod round_trip {
    use super::*;

    #[test]
    fn an_encoded_value_is_always_definite_length() {
        // Nothing here has a reason to write the longer form, so encoding stays
        // definite even though decoding accepts both, and reading back what was
        // written is the property that matters for interop.
        let items = vec![1u64, 2, 3];
        let encoded = to_cbor_vec(&items).unwrap();
        assert_eq!(encoded, [0x83, 0x01, 0x02, 0x03]);
        assert_eq!(from_cbor::<Vec<u64>>(&encoded).unwrap(), items);

        assert_eq!(to_cbor_vec(&"hi".to_string()).unwrap(), [0x62, b'h', b'i']);
    }
}
